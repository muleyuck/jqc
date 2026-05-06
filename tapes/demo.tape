Output demo.gif

Set Shell zsh
Set TypingSpeed 40ms

Set FontSize 32
Set Width 2200
Set Height 800

Hide

Type "source ~/.zshrc" Enter
Type "clear" Enter

Show

Sleep 500ms
Type "cat tests/fixtures/config.jsonc | jqc fmt"  Sleep 1s

Enter Sleep 1s

Type "jqc '.port' tests/fixtures/config.jsonc"  Sleep 1s

Enter Sleep 1s

Type "jqc set '.port' 8080 ./tests/fixtures/config.jsonc" Sleep 1s

Enter Sleep 1s

Sleep 3s
