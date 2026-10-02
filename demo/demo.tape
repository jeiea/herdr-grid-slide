# Run with `mise run demo`; `Key` lines are converted before VHS reads the tape.
# Each step types what it is about to do into the focused prompt, then clears it.
Output demo/demo.gif
Output demo/demo.mp4
Set Shell zsh
Set FontSize 14
Set Width 1280
Set Height 720
Set Framerate 24
Set Padding 12
Set TypingSpeed 35ms

Hide
Type "herdr --session demo" Enter
Sleep 3s
Show
Sleep 800ms

Type "# new-pane x5 (alt+n)"
Sleep 700ms
Ctrl+U
Key alt+n
Sleep 700ms
Key alt+n
Sleep 700ms
Key alt+n
Sleep 700ms
Key alt+n
Sleep 700ms
Key alt+n
Sleep 1500ms

Type "# focus (alt+h/k/l/j)"
Sleep 700ms
Ctrl+U
Key alt+h
Sleep 600ms
Key alt+k
Sleep 600ms
Key alt+l
Sleep 600ms
Key alt+j
Sleep 1s

Type "# move (alt+shift+h/k)"
Sleep 700ms
Ctrl+U
Key alt+shift+h
Sleep 1s
Key alt+shift+k
Sleep 1500ms

Type "# to-new-tab (alt+m)"
Sleep 700ms
Ctrl+U
Key alt+m
Sleep 1500ms

Type "# focus across tabs (alt+h/l)"
Sleep 700ms
Ctrl+U
Key alt+h
Sleep 1s
Key alt+l
Sleep 1s

Type "# move into prev tab (alt+shift+h)"
Sleep 700ms
Ctrl+U
Key alt+shift+h
Sleep 1500ms

Type "# exit: the rest rebalances"
Sleep 700ms
Ctrl+U
Type "exit" Enter
Sleep 1500ms

Type "# to-new-workspace (alt+shift+m)"
Sleep 700ms
Ctrl+U
Key alt+shift+m
Sleep 1500ms

Type "# focus across workspaces (alt+k/j)"
Sleep 700ms
Ctrl+U
Key alt+k
Sleep 1s
Key alt+j
Sleep 1s

Type "# new-tab, new-pane (alt+shift+n, alt+n)"
Sleep 700ms
Ctrl+U
Key alt+shift+n
Sleep 1s
Key alt+n
Sleep 1s

Type "# tab-to-new-workspace (ctrl+alt+m)"
Sleep 700ms
Ctrl+U
Key ctrl+alt+m
Sleep 1500ms

Type "# move-workspace-previous (alt+shift+i)"
Sleep 700ms
Ctrl+U
Key alt+shift+i
Sleep 1500ms

Type "# tab-to-next-workspace (ctrl+alt+j)"
Sleep 700ms
Ctrl+U
Key ctrl+alt+j
Sleep 2500ms
