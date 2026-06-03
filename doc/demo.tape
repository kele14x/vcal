# VHS tape for the README example.
#
# Regenerate the GIF with:
#   cargo build --release
#   vhs doc/demo.tape

Output doc/demo.gif

Set Shell "bash"
Set FontSize 18
Set Width 720
Set Height 380
Set Theme "GitHub Dark"
Set Padding 20
Set TypingSpeed 60ms

# Launch vcal silently so the recording starts at the first prompt.
Hide
Type "./target/release/vcal"
Enter
Sleep 500ms
Ctrl+L
Show

Type "reg [7:0] a"
Sleep 300ms
Enter
Sleep 400ms

Type "a = 4'hF + 4'hF"
Sleep 300ms
Enter
Sleep 400ms

Type "a"
Sleep 200ms
Enter
Sleep 900ms

Type "a + 4'b1"
Sleep 300ms
Enter
Sleep 1500ms
