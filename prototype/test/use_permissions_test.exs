defmodule Flow.UsePermissionsTest do
  use ExUnit.Case, async: true
  alias Flow.{Program, Runtime, Store, Ref, ID}
  @secret String.duplicate("payroll-secret", 4)
  @domain """
  [type ServiceID [id Service]] [type EmployeeID [id Employee]] [type JobID [id Job]]
  [type Money [decimal [range 0.01 1000000] [scale 2]]]
  [type QueueReceipt [record [field id JobID] [field state String]]]
  [entity Service [field id ServiceID] [field keyHash String [unique]]]
  [entity Employee [field id EmployeeID] [field salary Money] [field active Bool]]
  [auth [service Service [apiKey] [entity [eq Service.keyHash credential.hash]]]]
  [transport HTTP [auth service]]
  [seed Employee [rows [record [id "e1"] [salary 1000.00] [active true]]]]
  [permissions Service
    [Employee [USE [when target.active]]]
    [Employee.salary [USE [when target.active]]]
    [Payroll.compute [INVOKE [when true]]]
    [Payroll.send [INVOKE [when true]]]]
  [plugin Payroll
    [compute [input salary Money] [output Money] [pure] [release]]
    [send [input salary Money] [output Bool] [release]]]
  """
  @query """
  [query Compute [input employeeId EmployeeID] [output Money]
    [employee [entity $employeeId]]
    [result [invoke Payroll.compute [record [salary [use employee.salary]]]]]]
  """
  @queued """
  [mutate Send [atomic] [input employeeId EmployeeID] [output QueueReceipt]
    [employee [entity $employeeId]]
    [result [enqueue Payroll.send [record [salary [use employee.salary]]]
      [retry [attempts 2] [delay "1s"] [exhausted RETAIN]]]]]
  """

  defp options(source, send_handler) do
    [
      source: source,
      worker_interval: :manual,
      identities: %{
        "Service" => [
          %{
            "id" => "payroll",
            "keyHash" => :crypto.hash(:sha256, @secret) |> Base.encode16(case: :lower)
          }
        ]
      },
      plugins: %{
        "Payroll.compute" => %{
          mode: :pure,
          function: fn args -> Decimal.mult(args["salary"], Decimal.new("0.9")) end
        },
        "Payroll.send" => %{mode: :external, function: send_handler}
      }
    ]
  end

  test "USE permits only an explicit approved release, never a direct or derived output" do
    for result <- ["[use employee.salary]", "[mul [use employee.salary] 2]"] do
      assert {:error, %{code: :invalid_use}} =
               Program.compile(
                 @domain <>
                   "[query Leak [input employeeId EmployeeID] [output Money] [employee [entity $employeeId]] [result #{result}]]"
               )
    end

    no_release =
      String.replace(@domain, "[output Money] [pure] [release]", "[output Money] [pure]")

    assert {:error, %{code: :invalid_use}} = Program.compile(no_release <> @query)

    runtime =
      start_supervised!(
        {Runtime,
         options(
           @domain <>
             @query <>
             "[query Raw [input employeeId EmployeeID] [output Money] [employee [entity $employeeId]] [result employee.salary]]",
           fn _ -> true end
         )}
      )

    assert {:ok, result} =
             Runtime.execute(runtime, "Compute", %{"employeeId" => "e1"}, {"service", @secret})

    assert Decimal.equal?(result, Decimal.new("900.00"))

    assert {:error, %{code: :not_found}} =
             Runtime.execute(runtime, "Raw", %{"employeeId" => "e1"}, {"service", @secret})
  end

  test "queued release checks the source USE permission again before invoking a plugin" do
    owner = self()

    runtime =
      start_supervised!(
        {Runtime,
         options(@domain <> @queued, fn _, _ ->
           send(owner, :sent)
           true
         end)}
      )

    assert {:ok, _} =
             Runtime.execute(runtime, "Send", %{"employeeId" => "e1"}, {"service", @secret})

    assert {:ok, [%{state: "DONE"}]} = Runtime.process_jobs(runtime)
    assert_received :sent

    assert {:ok, _} =
             Runtime.execute(runtime, "Send", %{"employeeId" => "e1"}, {"service", @secret})

    store = :sys.get_state(runtime).store
    reference = %Ref{entity: "Employee", id: %ID{entity: "Employee", value: "e1"}}

    assert {:ok, :ok} =
             Store.transaction(store, &Store.update(&1, reference, %{"active" => false}))

    assert {:ok, [%{state: "BLOCKED"}]} = Runtime.process_jobs(runtime)
    refute_received :sent
  end
end
