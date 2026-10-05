defmodule Flow.ProgramTest do
  use ExUnit.Case, async: true
  alias Flow.Program

  @domain """
  [type UserID [id User]] [type PostID [id Post]]
  [type RequestID [id Request]]
  [type Count [integer [range 0 100]]]
  [entity User [field id UserID] [field subject String [unique]]]
  [entity Post [field id PostID] [field author User] [field count Count]]
  [type Summary [record [field id PostID] [field count Count]]]
  [auth [anonymous Anonymous]] [transport HTTP [auth anonymous]]
  """

  test "all application operations have checked contracts" do
    source = File.read!(Path.expand("../priv/workflows/application.flow", __DIR__))
    assert {:ok, program} = Program.compile(source)
    assert map_size(program.operations) == 11
    assert program.plugins["Payment.createUrl"].mode == :pure
    assert program.plugins["Email.sendConfirmation"].mode == :external
    assert program.plugins["Files.put"].mode == :transactional
    assert length(program.seeds) == 4
  end

  test "forward references are declarative and output selects only its named fields" do
    assert {:ok, _} =
             Program.compile(
               @domain <>
                 """
                 [query Find [input postId PostID] [output Summary]
                   [result post] [post [entity $postId]]]
                 """
             )
  end

  test "unknown bindings, unknown fields, mismatched brands and cycles are rejected" do
    for components <- [
          "[result missing]",
          "[result post.typo] [post [entity $postId]]",
          "[result a] [a b] [b a]",
          "[result [entity $userId]]",
          "[result [shell \"command\"]]"
        ] do
      source =
        @domain <>
          "[query Find [input postId PostID] [input userId UserID] [output Summary] #{components}]"

      assert {:error, %Flow.ValidationError{}} = Program.compile(source), source
    end
  end

  test "a query cannot hide a write in an unused binding" do
    source =
      @domain <>
        """
        [query Find [input postId PostID] [output Summary]
          [post [entity $postId]] [hidden [set post.count [as Count 2]]]
          [result post]]
        """

    assert {:error, %{code: :query_effect}} = Program.compile(source)
  end

  test "writes require atomic and every created field has the declared type" do
    for body <- [
          "[output Summary] [result [create Post [record [id [new PostID]] [author [entity $userId]] [count [as Count 1]]]]]",
          "[atomic] [output Summary] [result [create Post [record [id [new UserID]] [author [entity $userId]] [count [as Count 1]]]]]",
          "[atomic] [output Summary] [result [create Post [record [id [new PostID]] [author [entity $userId]] [count [as Count 1]] [extra true]]]]"
        ] do
      assert {:error, %Flow.ValidationError{}} =
               Program.compile(@domain <> "[mutate Add [input userId UserID] #{body}]")
    end
  end

  test "unsafe external invocation and duplicate routes are rejected" do
    source =
      @domain <>
        """
        [plugin Mail [send [output Bool]]]
        [mutate Send [atomic] [output Bool] [result [invoke Mail.send [record]]]]
        """

    assert {:error, %{code: :unsafe_effect}} = Program.compile(source)

    source =
      @domain <>
        """
        [query A [input postId PostID] [http GET "/posts/{postId}"] [output Summary] [result [entity $postId]]]
        [query B [input id PostID] [http GET "/posts/{id}"] [output Summary] [result [entity $id]]]
        """

    assert {:error, %{code: :duplicate_route}} = Program.compile(source)
  end

  test "malformed metadata returns a validation error" do
    for tail <- [
          "[output Summary] [result]",
          "[output Summary] [result [entity $postId]] [http]",
          "[output Summary] [result [entity $postId]] [websocket]"
        ] do
      assert {:error, %Flow.ValidationError{}} =
               Program.compile(@domain <> "[query Q [input postId PostID] #{tail}]")
    end
  end
end
