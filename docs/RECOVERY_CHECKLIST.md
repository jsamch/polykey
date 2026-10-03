# Recovery checklist

Print this page and keep it with the coordinator file. The bcp window shows the same text under
"Recovery checklist" (from Home and from Recover). This file is the one source of the text.

## Before you start

1. Decide who is present. Recovery needs the passcodes and enough plates; nobody should look over the shoulder.
2. Gather plates. You need any k shares of the same set (k is printed on every plate and in the manifest, for example "NEED 3"), or the master plate alone.
3. Gather the passcodes. The share passcode opens shares, the master passcode opens the master plate. They are kept apart from the plates.
4. Use a trusted computer that you can switch off afterwards. Take it off the network: unplug the cable, switch off Wi-Fi.
5. Have pen and paper ready to write the passphrase.

## Recover

6. Start bcp from the program file you stored (no installation, no Python). Do not open a browser or any other program.
7. Press "Recover" in the bcp window. Command line: run "bcp recover" in a terminal.
8. Add the plates. Type or paste each plate string, add photos or text files with "Add files", or drop them on the window. Either spelling of a plate string is fine, with spaces or with colons. Command line: "bcp recover plate1.jpg plate4.jpg plate5.jpg", or "bcp recover" alone to type them.
9. Wait until the set card shows "Ready". If several complete sets are listed, pick the set you want; each is recovered on its own.
10. Press "Recover passphrase". Enter the passcode when asked: the share passcode for shares, the master passcode for a master plate. You have three tries. A wrong passcode is reported after the key is rebuilt.
11. The passphrase is shown once. Type the no-space form exactly as shown into the vault as the master password, or write it down. Use the "Check what I wrote" box to compare your copy group by group.
12. Press "I have recorded it". The passphrase and the plates are wiped from the window.

## Afterwards

13. Close bcp. Command line: clear the terminal and its scrollback.
14. Put the plates back in their separate places and the passcodes back in their separate envelopes. If a passcode was said aloud or written anywhere else, change the set.
15. Switch the computer off. Note the date, who was present and which set ID was used, and file the note with the manifest.

## If something goes wrong

- "checksum mismatch" on a plate: the string was misread or the plate is damaged. Type the text side by hand, or use another plate.
- Wrong passcode three times: stop, check the envelope and the set ID (the share passcode and the master passcode differ), then start again.
- Not enough plates: add another plate of the same set. Plates of different sets do not mix.
- Never type the passphrase, a passcode or a plate string into a web page, a chat or a phone.
