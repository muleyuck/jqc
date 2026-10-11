# Run from the repository root after `cargo build`: vhs tapes/demo.tape
Output tapes/demo.gif

Set Shell zsh
Set TypingSpeed 40ms

Set FontSize 28
Set Width 2200
Set Height 800

Hide
Type `exec env -u JQ_COLORS -u NO_COLOR PATH="$PWD/target/debug:$PATH" zsh -f` Enter
Type `PS1='$ '` Enter
Type "clear" Enter
Show

Sleep 500ms
Type "jqc fmt tests/fixtures/config.jsonc" Sleep 1s
Enter Sleep 2s

Type "jqc '.port' tests/fixtures/config.jsonc" Sleep 1s
Enter Sleep 2s

Type "jqc --edit '.port = 8080' tests/fixtures/config.jsonc" Sleep 1s
Enter Sleep 4s
