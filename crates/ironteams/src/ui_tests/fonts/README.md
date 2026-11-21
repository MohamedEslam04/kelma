# Fonts for the UI tests

DejaVu Sans (regular and bold) 2.37, the default sans-serif font of the Linux CI runner (Ubuntu).
The UI tests on Windows and macOS use it (`ui_tests::init_platform` points `SLINT_FONT_PATH` at
this folder), so text is measured the same on every system and the width checks do not depend on
Segoe UI or San Francisco. Linux keeps the system's fontconfig default.

License: the Bitstream Vera / DejaVu license, `LICENSE-DejaVu.txt`.
