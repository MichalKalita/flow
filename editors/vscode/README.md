# Flow for VS Code

Syntax highlighting for Flow programs. The grammar colors comments, JSON strings and numbers, brackets, form heads, `$inputs`, types, and enum-style constants. It does not parse or validate the file.

## Install

Symlink this folder into the editor extensions directory and reload the window:

```sh
ln -sf "$(pwd)" "$HOME/.vscode/extensions/ha-basic-flow"
```

Cursor:

```sh
ln -sf "$(pwd)" "$HOME/.cursor/extensions/ha-basic-flow"
```

Open any `.flow` file after reload. The language mode should show **Flow**.
