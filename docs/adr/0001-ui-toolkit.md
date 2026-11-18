# ADR 0001: UI toolkit

- Status: proposed (pending the manual Arabic typing test)
- Date: 2026-11-18

## Context

kelma needs a Teams-like custom layout, correct Arabic and mixed Arabic/English (bidi) text, and
low memory: the goal is under 120 MB REAL at idle, against about 1.1 GB for the Teams web app.
GTK4 has the best RTL support, but it looks like GNOME rather than Teams. That left two
candidates: Slint 1.19 and iced 0.14.

## Spike

Both spikes live in `spikes/slint-spike` and `spikes/iced-spike`. Each builds the same screen
from `spikes/shared/mock.rs`:

- a left rail
- a chat list with unread badges
- 300 message bubbles mixing Arabic, English and mixed text, emoji and long paragraphs
- a composer

Measured on Fedora 44 / KDE Plasma (Wayland), release build, 10 s after launch.

| | Slint (femtovg, GPU) | Slint (software) | iced (wgpu) | iced (tiny-skia) |
|---|---|---|---|---|
| PSS memory | 111 MB | **26 MB** | 168 MB | 27 MB |
| Arabic shaping | correct | correct | correct | not inspected |
| Mixed bidi line order | correct | correct | correct | not inspected |
| RTL paragraph alignment inside bubble | correct | correct | **broken**: text drawn outside the bubble, edges clipped | not inspected |
| Text with emoji | renders (monochrome) | renders (monochrome) | **some messages blank** | not inspected |
| Out of the box with system dark theme | needs `Palette.color-scheme` | same | needs `Theme::Light` | same |

Emoji are monochrome in Slint because the system emoji font is Noto Color Emoji in COLRv1
format. A bundled color emoji font (COLRv0 or CBDT) will be evaluated in phase 3.

## Decision

Slint, run with the software renderer (`SLINT_BACKEND=winit-software`) by default.

- It is the only candidate that rendered all RTL text correctly.
- It has the lowest memory, about 26 MB for the whole UI.
- Its declarative `.slint` markup makes the Teams-like layout and styling straightforward.
- `ListView` virtualizes rows, which keeps long histories cheap.

The GPU renderer stays available as a fallback if scrolling is not smooth enough.

## Pending

The decision is confirmed only after the manual Arabic typing test passes in the Slint composer:

- typing with the Arabic keyboard layout
- mixed Arabic/English input
- selecting text
- copy and paste
- cursor movement across the text
