defmodule Flow.PolicyCheckerTest do
  use ExUnit.Case, async: true
  alias Flow.Program

  @domain """
  [type UserID [id User]] [type PostID [id Post]] [type FolderID [id Folder]]
  [type GrantID [id Grant]] [type DownloadID [id Download]]
  [type Remaining [integer [range 0 1]]]
  [entity User [field id UserID] [field name String]]
  [entity Folder [field id FolderID] [field parent [optional Folder]]]
  [entity Post [field id PostID] [field author User] [field public Bool] [field folder Folder]]
  [entity Grant [field id GrantID] [field user User] [field remaining Remaining]]
  [entity Download [field id DownloadID] [field grant Grant]]
  """

  test "policy types reject misspelled fields and nonboolean predicates" do
    for predicate <- [
          "actor.roles",
          "[eq target.typo actor]",
          "target.author",
          "[gt target.author actor]",
          "[can READ target.id]",
          "[eq before.public true]"
        ] do
      assert {:error, %Flow.ValidationError{}} =
               Program.compile(@domain <> "[permissions User [Post [READ [when #{predicate}]]]]")
    end
  end

  test "positive recursive inheritance is legal and negation over ordinary data remains legal" do
    assert {:ok, _} =
             Program.compile(
               @domain <>
                 """
                 [permissions User
                   [Folder [READ [when [and [ne target.parent null] [can READ target.parent]]]]]
                   [Post [READ [when [and [not target.public] [can READ target.folder]]]]]]
                 """
             )
  end

  test "negative cycles are rejected, including across inherited actions" do
    for policy <- [
          "[permissions User [Post [READ [when [not [can READ target]]]]]]",
          "[permissions User [Post [UPDATE [includes READ] [when [can DELETE target]]] [DELETE [when [not [can UPDATE target]]]]]]",
          "[permissions User [Folder [READ [when [not [can READ target.parent]]]]]]"
        ] do
      assert {:error, %{code: :negative_permission_cycle}} = Program.compile(@domain <> policy)
    end
  end

  test "negative permission dependencies without a recursive cycle are legal" do
    assert {:ok, _} =
             Program.compile(
               @domain <>
                 """
                 [permissions User [Folder [READ [when true]]]
                   [Post [READ [when [not [can READ target.folder]]]]]]
                 """
             )
  end

  test "wildcard ownership can compare any authenticated identity with a domain owner" do
    assert {:ok, _} =
             Program.compile(
               @domain <> "[permissions * [Post [READ [when [eq actor target.author]]]]]"
             )
  end

  test "single-use grants declare exact before and after changes" do
    assert {:ok, _} =
             Program.compile(
               @domain <>
                 """
                 [permissions User
                   [Grant [UPDATE [when [and [eq before.user actor]
                     [eq before.remaining 1] [eq after.remaining 0] [only changed remaining]]]]]
                   [Download [CREATE [when [updates transaction after.grant
                     [before [remaining 1]] [after [remaining 0]]]]]]]
                 """
             )

    assert {:error, _} =
             Program.compile(
               @domain <>
                 """
                 [permissions User [Download [CREATE [when [updates transaction after.grant
                   [before [remaining 1]] [after [typo 0]]]]]]]
                 """
             )
  end
end
