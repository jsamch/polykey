# Acceptance results

Template for the manual checks of WORKPLAN step 6.8 and for Phase 8 (field acceptance). Fill in
the Result column with PASS, FAIL or N/A and use Notes for what was seen (build, date, tester,
screenshots, anything odd). Leave a row blank until it has been tried. Use a DEMO set and test
vectors only; never a real key or real passcodes.

Build under test: ______________________  Commit or release hash: ______________________

Tester: ______________________  Date: ______________________

## Step 6.8: manual UI checklist

The same rows apply to every platform. Copy a table per machine tested.

### Windows 10

| Check | Result | Notes |
| --- | --- | --- |
| Window opens | | |
| Double-click launch shows no console | | |
| `bcp recover` from a terminal still prompts | | |
| High DPI 100 percent | | |
| High DPI 150 percent | | |
| High DPI 200 percent | | |
| Dark theme | | |
| Light theme | | |
| Keyboard-only full flow (Create, Check, Recover, no mouse) | | |
| Create DEMO set | | |
| Check | | |
| Recover | | |
| Self test | | |
| File dialog opens (or typed path fallback works) | | |
| Screen reader (Narrator): deferred, AccessKit is off (DECISIONS entry 7) | | |

### Windows 11

| Check | Result | Notes |
| --- | --- | --- |
| Window opens | | |
| Double-click launch shows no console | | |
| `bcp recover` from a terminal still prompts | | |
| High DPI 100 percent | | |
| High DPI 150 percent | | |
| High DPI 200 percent | | |
| Dark theme | | |
| Light theme | | |
| Keyboard-only full flow (Create, Check, Recover, no mouse) | | |
| Create DEMO set | | |
| Check | | |
| Recover | | |
| Self test | | |
| File dialog opens (or typed path fallback works) | | |
| Screen reader (Narrator): deferred, AccessKit is off (DECISIONS entry 7) | | |

### macOS arm64

| Check | Result | Notes |
| --- | --- | --- |
| Window opens | | |
| Double-click launch shows no console (not applicable on macOS unless a terminal appears) | | |
| `bcp recover` from a terminal still prompts | | |
| High DPI 100 percent | | |
| High DPI 150 percent | | |
| High DPI 200 percent | | |
| Dark theme | | |
| Light theme | | |
| Keyboard-only full flow (Create, Check, Recover, no mouse; Cmd+1 to Cmd+5 or Ctrl+1 to Ctrl+5) | | |
| Create DEMO set | | |
| Check | | |
| Recover | | |
| Self test | | |
| File dialog opens (or typed path fallback works) | | |
| Screen reader (VoiceOver): deferred, AccessKit is off (DECISIONS entry 7) | | |

### Linux X11

| Check | Result | Notes |
| --- | --- | --- |
| Window opens | | |
| Double-click launch shows no console (not applicable on Linux) | | |
| `bcp recover` from a terminal still prompts | | |
| High DPI 100 percent | | |
| High DPI 150 percent | | |
| High DPI 200 percent | | |
| Dark theme | | |
| Light theme | | |
| Keyboard-only full flow (Create, Check, Recover, no mouse) | | |
| Create DEMO set | | |
| Check | | |
| Recover | | |
| Self test | | |
| File dialog opens (or typed path fallback works) | | |
| Screen reader (Orca): deferred, AccessKit is off (DECISIONS entry 7) | | |

### Linux Wayland

| Check | Result | Notes |
| --- | --- | --- |
| Window opens | | |
| Double-click launch shows no console (not applicable on Linux) | | |
| `bcp recover` from a terminal still prompts | | |
| High DPI 100 percent | | |
| High DPI 150 percent | | |
| High DPI 200 percent | | |
| Dark theme | | |
| Light theme | | |
| Keyboard-only full flow (Create, Check, Recover, no mouse) | | |
| Create DEMO set | | |
| Check | | |
| Recover | | |
| Self test | | |
| File dialog opens (or typed path fallback works) | | |
| Screen reader (Orca): deferred, AccessKit is off (DECISIONS entry 7) | | |

### What the keyboard-only row covers

Tab and Shift+Tab move between controls and the first useful field has the focus when a screen
or step opens. Enter goes to Next in the wizard and runs Run checks, Recover passphrase and Run
self test when nothing else has the focus, and answers the passcode dialog. Create the set, "I
have recorded it" and "Leave and wipe" need a click or Space; a bare Enter on them does
nothing. Escape cancels the passcode dialog, answers Stay on the leave question and keeps
locking on. Ctrl+1 to Ctrl+5 open Home, Create, Check, Recover and Self test.

## Phase 8: field acceptance

Only after this phase passes is the tool used to generate a real set, on the offline machine,
from a signed release (WORKPLAN Phase 8).

| Item | Result | Notes |
| --- | --- | --- |
| Clean offline Windows VM: download release, verify hash, run `bcp selftest` and the GUI self test | | |
| Generate a DEMO set with the GUI, engrave on the target material, photograph with two phones | | |
| Verify the engraved DEMO set with the GUI | | |
| Verify the engraved DEMO set with the Rust CLI | | |
| Verify the engraved DEMO set with the Python script | | |
| Recover the engraved DEMO set with the GUI | | |
| Recover the engraved DEMO set with the Rust CLI | | |
| Recover the engraved DEMO set with the Python script | | |
| Photos added to `tests/photos/` | | |
| Recover a Python-generated DEMO set with the GUI from photos | | |
| Results recorded in this file | | |
