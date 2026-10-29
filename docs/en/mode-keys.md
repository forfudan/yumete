# Keys in input and select modes

These are the default bindings. For Normal-mode keys, see the
[movement](manual/06-moving.md), [selection](manual/07-selection.md), and
[changing](manual/08-changing.md) references.

## Insert mode

<!-- help: i_Esc -->
`Esc` leaves Insert mode and returns to Normal mode.

<!-- help: i_Enter -->
`Enter` inserts a newline; in Markdown a list marker is continued.

<!-- help: i_Backspace -->
`Backspace` deletes the preceding grapheme.

<!-- help: i_Tab -->
`Tab` inserts spaces or a tab according to `editor.tab_inserts`.

## Select/extend mode

<!-- help: v_; -->
`;` collapses the selection. See [selection](manual/07-selection.md) for
extending and keeping selections.

<!-- help: v_Esc -->
`Esc` returns to Normal mode.

## Command-line editing

<!-- help: c_Backspace -->
`Backspace` deletes the preceding character; at the start of an empty prompt it
closes the command line.

<!-- help: c_Enter -->
`Enter` runs the command.

<!-- help: c_Esc -->
`Esc` cancels the command line.

<!-- help: c_C-w -->
`C-w` deletes the preceding word.

<!-- help: c_C-u -->
`C-u` clears the command line.

<!-- help: c_Tab -->
`Tab` cycles through completions. See [command-line editing](manual/74-commands-remember-the-verb-not-the-argument.md).
