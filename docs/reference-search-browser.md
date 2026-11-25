# Reference: the Search browser

Search is a Mac browser by Office Commun (Paris), at [searchbrowser.app](https://searchbrowser.app)
with its source at [github.com/driceroland/Search](https://github.com/driceroland/Search). The
owner asked for it as a reference again on 2026-11-25, beside Arc (commit af51b8c had dropped
it). Post 1 in [inspiration.md](inspiration.md) is this browser's settings. This file records
what it does, its measurable look, how its source gets there, and where each item lands in Lynk
and nd. We study and write our own: nothing here is code or an asset to paste.

Read from: every page of the site (home, From Arc, iPhone, changelog, roadmap, measured, FAQ,
privacy, sponsors), the source at `d052f5b` (version 1.0.20, 24 November 2026), and the
notarized 1.0.20 build run on the macbook in a scratch home. Paths below are relative to
`Sources/Search/` in that repository unless they say otherwise.

## Facts

| | |
| --- | --- |
| Engine | WebKit: `WKWebView`, the system framework, one per tab, built lazily (README "How it's put together", `Tab.swift`). Not Chromium, not CEF, not Electron |
| Language | Swift 6 toolchain in Swift 5 language mode, about 42,000 lines in the app target (`Package.swift`); a little Objective-C, C++ (`Engine/search-ai-engine.cpp`, an optional local AI helper) and injected JavaScript |
| UI toolkit | SwiftUI for everything drawn, AppKit for the title bar, window dragging, panels and the split container (README, `App.swift`, `PaneStage.swift`) |
| Platforms | macOS 14 or later, Apple silicon and Intel (separate Intel download). Extensions need macOS 15.4 (`WKWebExtension`). An iPhone app is in TestFlight beta. No Linux, no Windows |
| Packaging | SwiftPM executable assembled into `Search.app` by `build.sh`, Developer ID signed and notarized (`publish.sh`), shipped as a 7.7 MB DMG (14 MB installed) and a Homebrew cask (`driceroland/tap/search`). Self-update reads a signed feed hourly and swaps the bundle for the next launch (`Updater.swift`) |
| Dependencies | None beyond Apple frameworks |
| Extensions | Chrome Web Store CRX3 fetched and signature checked (`Crx.swift`), run on `WKWebExtension`, missing Chrome APIs filled by an injected shim answered natively (`ExtensionShims.swift`) |
| Licence | MIT, copyright 2026 Office Commun (`LICENSE`). Optional developer tools packages adapt Lighthouse (Apache 2.0), Speedline (MIT) and Chromium DevTools data (BSD 3-Clause), listed in `THIRD_PARTY_NOTICES.md` |

What the licence permits: MIT allows copying, changing and redistributing with the copyright
notice kept. It is not copyleft, so reading the source places no obligation on Lynk. The
constraint is ours: we copy no code, no strings, no icons, no murals and no copy from the site,
because Lynk is written from its own specs and a pasted block would carry Office Commun's notice
with it. Numbers (sizes, timings, colours) are facts we measure and may choose to match.

Lynk is Chromium through CEF on nd, so the engine-side techniques here (content rule lists,
`takeSnapshot`, `WKWebExtension`) do not transfer; the look and the interaction do.

## Captures

In [reference/search-browser/](reference/search-browser/), from the 1.0.20 build on the macbook
(macOS 27, Reduce Motion on, so motion was read from source rather than seen):

| File | What it shows |
| --- | --- |
| `welcome-dark.png` | First launch: the welcome, two switches in an inner card, page dots, Skip and Continue |
| `sidebar-rest-dark.png` | Column at rest: selected row filled, cross on it, speaker on the playing tab, quiet "New tab" row, two foot glyphs |
| `sidebar-hover-dark.png` | Close-up: hovered row with its round cross, selected row, letter marks where favicons are missing |
| `sidebar-rest-light-inactive.png` | Light, app inactive: the hand-drawn grey traffic lights, the quieter fills |
| `sidebar-folded-dark.png` | ⌘S: column folded, page edge to edge with square corners |
| `edge-reveal-dark.png` | Pointer at the left edge: the column back over the page as a floating card |
| `top-tabs-dark.png`, `top-tabs-light.png` | ⇧⌘S: fixed-width tabs in the 52 pt strip, the split pair as one item |
| `new-tab-field-light.png` | ⌘T: the one field centred on a blank tab, hovered row with its cross |
| `field-over-page-dark.png` | ⌘L over a page: page dimmed with ground, one suggestion row hanging under the field |
| `tab-list-cmdk-dark.png` | ⌘K: open tabs by name, most recent first, address in muted type |
| `switcher-dark.png` | ⌃Tab: page pictures, one sliding highlight, the ⌃W ⌃R ⌃M legend |
| `switcher-after-restart-light.png` | ⌃Tab after a relaunch: tabs with no picture yet show their letter on a plain card |
| `split-new-pane-dark.png` | ⌥⌘N: the new pane's field with "Or bring in an open tab" |
| `split-pair-in-column-dark.png` | A split pair in the column: two rows inside one outlined group |
| `settings-find-dark.png` | Settings beside a page, "Find a setting" for "dark": breadcrumbs, counts on the rail |
| `settings-appearance-dark.png`, `settings-appearance-light.png` | Appearance: Light and Dark thumbnails, Tabs layout thumbnails, one sentence per row |
| `little-window-dark.png` | A link from another app in the small window, "Open in Search" at its top |
| `floating-video-rest.png`, `floating-video-hover.png` | ⇧⌘P: the floating window at rest and with its controls |
| `floating-video-tab-placeholder-dark.png` | The tab the video left: "This page is playing in the floating window." |
| `reader-light-in-dark-frame.png` | ⇧⌘R with the window dark: the article stays light by default |
| `reader-corner-switch-light.png`, `reader-dark-article.png` | With "Reading mode follows light and dark" on: the corner switch, and the dark article |

Peek, the space swipe and the tinted themes were not captured: peek needs its setting turned
on (off by default, and settings could not be changed by script mid-run), and the swipe needs a
trackpad.

## The design system

Colour, shape and motion each come from one small set of tokens. Matching them changes the
most for the least code.

### Colour

- Every colour is a light and dark pair of plain greys, no hue (`Design.swift:33-44`): ground
  1.0 / 0.11 (white level), ink 0.09 / 0.93, muted 0.55 / 0.58, faint 0.83 / 0.32, hairline
  0.91 / 0.20, wash (selected row) 0.937 / 0.175, hover 0.965 / 0.15, frame (the window ground
  around the page card) 0.955 / 0.075. One `NSColor` per token resolves by appearance; SwiftUI
  wraps the same object, so nothing else in the code knows which mode it is in.
- Chrome greys are drawn as black (light) or white (dark) at an alpha, chosen so they land on
  the same grey as the opaque token over a plain ground (`Design.swift:62-116`). Over a tinted
  space they take the tint instead of reading as grey patches. On the frame ground: selected row
  0.0628 / 0.0865, hover 0.0419 / 0.0595, pin tile 0.0314 / 0.0432, live pin 0.0995 / 0.1243,
  muted ink 0.4241 / 0.5459. The set is passed down the view tree, picked by which ground the
  column sits on.
- The accent appears in two places only: the 2 pt loading line while the column is folded and
  the capture outline. Selection, hover and focus are all neutral.
- Status colours: safe green-700 / green-400, unsafe amber-700 / amber-400.

### Type

System font (SF) at fixed sizes, regular or medium, semibold only for titles. Tab title 12.5
(14 in the roomy setting), settings row title 13 with detail 11.5 muted and 3 between, sheet
title 17 semibold as a card or 22 as a tab, field text 15.5, suggestion title 13 with its
address 12 muted, small captions 10.5 to 11.5. Letter marks are size x 0.56 medium.

### Shape and space

Radii: page card 10, rows, pills and tiles 9, toolbar buttons 8, inner cards 11, peek and
switcher 12, field and suggestion list 14, sheets 16. Always the continuous (squircle) corner.
Page card gap 8. Rows 28 tall (36 roomy), 2 apart, 10 in from the column edge. Column 232
default, 176 minimum, 440 maximum, icons-only below 200. Field 560 wide.

### Depth

One floating recipe for every surface over the page: ground fill, a 1 pt hairline, continuous
corners, a soft black shadow (field 0.06 radius 24 y 8, list 0.07 r 20 y 6, sheet 0.16 r 34
y 12, peek 0.25 r 30 y 10), and a dim behind it that grows with how modal it is: 0.10 black for
sheets, 0.12 for the switcher, 0.22 for peek, and ground at 0.74 for the field raised over a
page. No materials except the site card (`.menu` vibrancy) and an opt-in window glass. The page
card itself has no soft shadow: a hairline of exactly one device pixel, edges snapped to whole
pixels (`Design.swift:565-629`), because a shadow read as a grey ring.

### Motion

Three curves for the whole app (`Design.swift:294-426`):

| Name | Value | Used for |
| --- | --- | --- |
| glide | spring, response 0.34, damping 0.82 | things moving between two places: the selected-row pill, editing |
| settle | spring, response 0.30, damping 0.86 | arrivals, departures, reorders, drops |
| quick | ease-out 0.14 s | hover, colour, opacity, small state flips |

- One gate: if macOS Reduce Motion or the app's own "Reduce animations" is on, every curve
  returns no animation. The app setting also zeroes timed fades.
- Low Power Mode or a serious thermal state (opt-in "light motion") swaps both springs for a
  0.16 s ease-out.
- Press feedback is an opt-in scale of 0.97, down on quick, back on settle.
- Hover colour changes on rows are not animated.
- Endless motion (loading rings, the field's breathing shadow, skeleton sheen) runs as Core
  Animation layers in the render server, not in the view tree, and does not start under Reduce
  Motion. The note in `Omnibox.swift:422-503` measured the SwiftUI version at 18% of a core.

## Window and column

**Frame.** No toolbar and no title band. Tabs down the left on the frame grey, the page a card
inset 8 from the window edges and the column, radius 10. Folded (⌘S) the card goes edge to edge
with square corners. Tabs across the top (⇧⌘S) put the page full bleed under a 52 pt strip (36
compact). Technique: a titled, full-size-content window with a transparent, hidden title; no
`NSToolbar` on purpose, because on macOS 26 a toolbar raises the window corner radius from 17.5
to 31.5 (`Lights.swift:6-10`). The title bar container is resized to the 52 pt strip so the
traffic lights centre in it (close button centre at 26, 26), and they are re-placed after every
resize, key change and full-screen exit. With the app inactive, grey ovals are drawn over the
lights because the system draws them near white (`Stage.swift:1362-1386`).

**Clipping the page.** The page is laid out once at its final size and only the clip animates:
an animatable SwiftUI shape pushes per-frame insets into the AppKit stage, which sets the web
view layer's continuous corner radius, or a cached mask layer while the card is smaller than the
page (`Design.swift:659-726`, `Stage.swift:378-455`). Offsets are rounded to device pixels so
the page is never resampled soft.

**Fold and edge reveal.** ⌘S grows the page first, then slides the column off. With it folded,
the pointer at the left edge (6 pt zone, 48 pt of overshoot still counts) slides it back over
the page as a floating card (8 gap, radius 10, shadow black 0.14 radius 20 cast from a shape
behind the rows), traffic lights riding in its corner frame by frame. It leaves 65 ms after the
pointer does. Out: spring 0.14 / 0.86; in: spring 0.13 / 1.0, tuned against Arc and Dia to
about 95 ms and 70 ms to 90% travel. An optional Arc-like slide is ease-out 0.26 s and ease-in
0.22 s. Under Reduce Motion it crossfades in place over 0.2 s (`Fold.swift:136-158, 376-406`).

Lynk: `docs/sidebar.md` "Hiding and edge reveal" has the same behaviour with 200 ms in, 150 ms
out, ease-out, and a 250 ms grace; parity boxes "Shift+Cmd+B hides" and edge reveal in
Windows and layout. Search's numbers are faster and its grace a quarter as long. Lane:
`sidebar-polish`. nd: `scaffold().edge_reveal()` exists; it needs the spring values as options
or tokens, not new code.

## Tab rows

What the user sees: a full-width line, favicon then title. The selected row has a soft grey
ground; others are bare until the pointer is over them. A cross appears at the right on hover
and the title fades under it rather than shortening.

- Row 28 tall, 2 apart, radius 9, 10 in from each side, favicon 15 with corner size x 0.22,
  title 12.5 regular, 8 between icon and title (`Side.swift:2048-2350`, `Prefs.swift:303-304`).
- Title ink has three levels: selected in full ink, hover at ink 0.7, rest in muted.
- Close: an 8 pt semibold cross in a 15 pt circle of ink 0.07, its hit area 30 x the row
  height, shown on hover only. The title is masked by a 16 pt linear fade then 26 pt clear at
  the right edge while hovered, so it keeps its length and nothing reflows
  (`Side.swift:2184-2195`).
- Loading: a 10 pt ring, 1.4 line, 78% of a turn, one turn per 0.85 s, as a Core Animation
  shape layer (`TabBar.swift:2460-2530`).
- Audio: speaker glyph 8 pt, click mutes; on hover it shifts 23 pt in to sit beside the cross.
- Sleeping tabs dim to 0.45 only when "Dim sleeping tabs" is on; pin letters always dim.
- Holding ⌘ swaps the end slot for the tab's digit, 11 pt medium monospaced.
- A press selects on mouse down; a click on the selected row edits its address.
- Insert and remove: scale 0.94 anchored at the leading edge plus opacity.

**The pill that glides.** The selected row's ground carries one matched-geometry id per space;
when the selection changes the ground flies from the old row to the new one on glide
(`Side.swift:437, 1836-1876`). The same id is on the selected pin square, so it glides between
pins and rows and between the halves of a split. A space switch disables the animation for that
transaction so it never glides across spaces. The site's line: "the grey slides to the tab you
pick".

Lynk today: rows 28 tall, 2 apart, radius 8, favicon 16, selected fill ink 0.12, hover ink 0.06,
asleep 0.5, close in a 20 pt box (`crates/lynk/src/sidebar.rs:19-51`); the fill jumps between
rows and the title is not faded. Lynk's fills are about twice Search's (0.063 / 0.035 over
white). Parity: Sidebar, "Tab rows: 28 pt tall...", "A row's trailing slot", "Sleeping tabs
draw dimmed". Lane: `sidebar-polish`, or the shell lane now since it owns the row. nd: the
glide is `hero_from` (`nd-ui/src/element.rs:579`), not yet proven between sibling rows on
every backend; the title fade needs a linear gradient mask, which nd's element API does not
expose (gpui has `linear_gradient` internally, nd-ui and nd-components do not).

## Pins

- Squares: 34 x 34 cells, 4 apart, at most four in a row, rows balanced with the fuller ones
  first (5 is 3 + 2, 7 is 4 + 3, 10 is 4 + 3 + 3); one row keeps three slots even with one or
  two pins. Wider cells become shorter buttons, never bigger icons: height is min(34, cell
  width), and every inner size scales from the square (radius s x 9/34, icon s x 16/34, letter
  s x 12/34 medium) (`Side.swift:1049-1150, 1888-2030`).
- Letter or favicon per setting; the letter is the user's to edit in place.
- Colours: rest tile grey, hover the hover grey, selected the darker live-pin grey with the same
  gliding id as rows.
- Pinned rows (Arc's pinned list) sit above a keep line: an 18 pt band with a 1 pt hairline;
  on hover it shows "Clear" to close every loose tab below (`Side.swift:2396-2440`).
- Back to pinned page: on a pin that wandered off its URL, hovering its favicon turns it into a
  back arrow; a click goes home. A double-click on the selected square does the same.
- ⌘W on a pin puts it to sleep instead of closing it.

Lynk: `docs/sidebar.md` pins are the tile grid (40 minimum, aspect 0.74, Liquid Glass on mac);
parity "Pinned tiles", "Reset to Pinned Page". Worth taking: balanced rows, the hover-to-go-home
icon, the keep line's Clear. Lane: `pins-profiles` (running). nd: `tile_grid` lays out evenly
filled rows; balanced row filling would be a `tile_grid` option.

## Drag

- Reorder starts after 5 pt; the carried row follows the pointer 1:1 with no animation of its
  own, and only the neighbours move, on settle, once its edge passes a neighbour's middle
  (`TabBar.swift:1305-1580`). Lifted look: shadow black 0.14 radius 12 y 4. Escape puts it back.
- Pin squares reorder in 2D to the nearest cell centre, shadow 0.16 radius 10 y 3.
- Crossing between pins and tabs opens a ghost slot in the target block (hover fill, ink 0.16
  outline, content at 0.55) while the carried item drops to 0.4, with a haptic tick when the
  target first appears. A drop lands exactly where the ghost was (`PinDrop.swift`).
- Into pin folders, rows do not move live: a 2 pt accent capsule marks the slot.
- Onto a page: only a band of max(120, 24% of the width) at each edge splits, and only after the
  pointer rests (under 6 pt of movement for 0.09 s); 20 pt outside the band lets go
  (`TabDrag.swift:114-132`).

Lynk: `docs/sidebar.md` "Tabs and drag" (a line marks the slot; tiles slide 0.2 s ease-out);
parity Sidebar drag boxes. What Lynk lacks is the live displacement and the carried row that tracks the pointer 1:1.
Lanes: `pins-profiles` now, `sidebar-organise` for drop to pin. nd: `source_list` drag marks a
line only; spring displacement and ghost slots are the "reorderable list with spring
displacement" row already in inspiration.md's nd table.

## Tabs across the top

Fixed width tabs, not title widths: 186 ideal, shrinking evenly to 36; under 104 a tab shows its
icon only; under 36 the row scrolls. Parts: 11 lead, 15 icon, 6 spacing, 15 close slot, 7
trail. Pins 30 wide; the plus appears only with the pointer near (opacity and scale 0.7 to 1 on
settle); 40 pt of empty strip kept after it as a drag handle (`TabBar.swift:245-266, 694-723`).
Lynk: the `compact` lane's layout and `layouts-themes`. nd: none.

## Spaces

- A space is a set of tabs and pins with an SF Symbol icon (24 to choose from) and an optional
  tint. The space button is 26 x 26, radius 8, filled with the tint at 0.14 (0.24 on hover).
  After a switch a name toast (capsule, 11.5 medium) shows 30 pt above it for 1.4 s.
- Tint: the space colour over the column, strip, bookmarks bar and the ground round the card,
  never the page, at 0.07 light and 0.11 dark (Rich 0.18 / 0.22, Vivid 0.30 / 0.40 with chroma
  raised). Two or three colour gradients are mixed in OKLab with chroma interpolated apart so the
  middle does not go grey, drawn as 24 stops across the whole window so column and frame are one
  gradient, with an optional 128 px noise grain at 0.035 / 0.04 (`SpaceGradient.swift`). Twelve
  swatches at OKLCH L 0.70 C 0.14, plus Graphite and Sand.
- Switching: two fingers sideways over the tabs. The axis locks after 6 pt; rows track the
  fingers 1:1; it commits past 50 pt or on a flick (12 pt at over 400 pt/s, speed from the last
  0.08 s). At the ends travel is divided by 4. Past the last space a "New Space" card comes in at
  0.6 x travel and needs twice the distance. On release a critically damped spring (omega 34)
  takes over from the finger's velocity and never overshoots. A stroke during the settle grabs it
  where it is. ⌃1 to ⌃9 run the same spring; the tint changes as soon as the slide starts
  (`SpaceSwipe.swift:490-1060`).
- Reduce Motion: the switch is instant.

Lynk: `crates/core/src/spaces.rs` exists; no parity section yet. Lane: `spaces`. nd:
`PageSwipe` and `on_swipe_page` give progress and commit; a background gradient with grain and a
window-wide tint are not in nd (ThemeRefinement tints accents, not surfaces by gradient).

## Command bar (the one field)

- One field, centred on a blank tab or raised over the page with ⌘L: 560 wide, 50 tall (22 text
  plus 14 above and below), padding 22, radius 14, ground, hairline, shadow 0.06 r 24 y 8; lifted
  60 pt above centre because dead centre reads low (`Omnibox.swift:28-169`). An option puts it at
  24% of the height.
- Over a page, the page is covered by ground at 0.74, not black.
- The list hangs 8 below the field in an overlay so the field never moves as rows come and go:
  padding 6, radius 14, shadow 0.07 r 20 y 6. Rows: 10 between icon and text, padding 12 by 9,
  title 13, address 12 muted, hint 12 muted. The keyboard row has the wash fill and the hovered
  row the hover fill: hover never moves the selection.
- Enter: scale 0.97 plus opacity; list scale 0.98 from the top plus opacity, on quick.
- A refused address shakes: three swings of 7 pt decaying over 0.5 s, off under Reduce Motion.
- Tab to search a site: typing the start of a known site and Tab turns it into a chip (favicon
  14, name 13 medium, wash fill radius 7); Backspace on empty removes it.
- Breathing shadow under the field on a blank tab: scale 0.97 to 1.03, opacity 0.65 to 1, 2.6 s,
  autoreversing, in Core Animation only.
- Nothing typed leaves the Mac before Return unless engine suggestions are turned on (off by
  default, 120 ms debounce, 0.8 s timeout, never in private tabs).
- A launcher option raises the same field over any app from a global key, in a non-activating
  panel at 18% from the top of the screen under the pointer (`Launcher.swift`).

Lynk: `docs/omnibox.md` and parity Command bar. Lynk specs 640 wide at 18%, ten 40 pt rows,
accent fill on the highlighted row, about 15% dim, no animation. Search is narrower, quieter
(neutral wash, not accent), dims with ground rather than black, and anchors the list so the
field never moves. Lane: `command bar` (running). nd: `panel()` and `cover()` exist; nothing new.

## Tab switcher (⌃Tab)

Hold ⌃, press Tab: after 0.15 s (so a quick flip never flashes) a panel of page pictures in most
recently used order, frozen for the gesture. Up to five columns, cards at most 176 wide, picture
height (width - 16) x 0.62, gaps 8, panel padding 12, radius 12, over a 0.12 black dim. One faint
rounded highlight slides between cards on glide instead of each card lighting up. Release ⌃
commits; ⌃W closes, ⌃M mutes, the pointer picks by hover. Pictures are `takeSnapshot` at 180 pt
wide, taken 0.4 s after you leave a tab and kept for the last ten; while the switcher is up,
fresh ones are requested 120 ms after the selection settles, staggered 40 ms
(`TabSwitcher.swift:139-672`, `Tab.swift:1797-1816`).

Lynk: inspiration section 4; lane `switcher-link-window`. nd: page snapshots from CEF have no
public API (`nd-webview/src/handle.rs`); modifier release is not exposed as an event on every
backend (gpui's `on_modifiers_changed` is vendored but not wrapped in nd-ui).

## Peek

Shift-click (configurable) opens a link in a card over the page: 0.82 x 0.86 of the page area,
radius 12, hairline, shadow 0.25 r 30 y 10, over a 0.22 black dim; opens on settle with scale
0.98 plus opacity, closes on quick. A column of three 28 pt round buttons outside its right edge:
close, open as a tab (⌘O or ⌘Return), keep beside this page (⌥⌘Return). A skeleton appears only
if the page has not painted after 150 ms. A pinch grows it to full size on a spring 0.3 / 0.92
and it becomes a tab. The peek is already a real tab outside the row, so promoting it is an
insert and select with no reload (`Peek.swift:12-372`). Pins can be set to keep their site: a
link to another site from a pin opens as a peek.

Lynk: inspiration section 5; lane `peek`. nd: overlay above a web view is guaranteed
(`nd-ui/src/overlay.rs:5-8`), pinch exists (`on_pinch`).

## Split view

Two to four pages side by side, 7 pt gutter with a centred grip (4 wide, 28 long, 40 when lit),
narrowest page 250 pt, minimum share 0.2. It snaps to even within 15 pt and lets go past 20 pt,
with a haptic tick. During the drag the divider resizes the real web views synchronously in one
AppKit container that is never rebuilt, and publishes the result once on mouse up; the slot
shows the page's under-page colour for 0.5 s after the last resize to hide WebKit's late repaint
(`PaneStage.swift:132-645`). The focused pane has an outline and gets page commands; each pane
is its own card. In the column a pair is stacked rows joined by hairlines; in the strip one item
with two halves.

Lynk: inspiration section 6; lane `split`. nd: `split()` with `on_resize` exists; the snap and
haptic are app logic on `nd-gesture::Haptic`.

## Settings

A tab (⌘,) or a 720 x 540 card: a 196 pt rail and a content column 600 wide. Rail items 30
tall, 13 pt (medium when selected), the selected one a ground pill with a faint shadow that
slides on spring 0.45 / 0.72. "Find a setting" (⌘F) at the top of the rail. One index lists
every row (title, detail, synonyms, the defaults keys it writes, the page and group); pages draw
from it, and search requires every typed word somewhere in a row's text. Results jump to the
row, scroll it to centre and highlight it for 1.4 s. A row that depends on another setting is
hidden in its page and shown dimmed in search with the sentence it needs
(`SettingsIndex.swift:23-880`, `Settings.swift:98-456`). Rows: title 13, detail 11.5 muted,
padding 14 by 11. Most Arc habits are off until turned on.

Lynk: inspiration section 1; parity "Settings window shows every setting as a native row";
`crates/core/src/settings_registry.rs` already exists, which is the same single-index idea.
Lanes: the settings tab (running), `settings-design`. nd: `settings_form`, `segmented`,
`search_field` exist; a thumbnail picker does not.

## Floating video

One of the two things the site advertises beside the size ("Leave YouTube or Netflix and the
video follows, above every app").

What the user sees: ⇧⌘P, or switching to another tab, another space or another app,
lifts the playing video into a small window at the bottom right of the screen that stays above
every app and every desktop. The tab it came from says "This page is playing in the floating
window." in 13 pt muted on the ground. Going back to the tab puts the video back in its page.
With the window still in view on another screen, or beside the app you went to, nothing floats
until the window is covered, minimised or on another desktop. Settings › General: "Float the
video on its own" (on), "The floating video" as Search's window or macOS picture in picture,
"Minimize the floating video". A site card switch, "Don't Float Videos Here", holds one site
back from floating on its own; ⇧⌘P still works there.

Measurable look (`Float.swift`):

- 440 x 247 at the bottom right, 24 from the screen edges; size and place remembered; aspect
  ratio locked; minimum 260 x 146; black ground, radius 14, clipped.
- No window shadow: WindowServer GPU measured 28% with one and 16 to 20% without.
- Controls appear on hover and fade in 0.16 s: close, tuck and back to tab as 30 pt round
  buttons along the top (14 and 52 from the left, 44 from the right), play and pause 50 pt in
  the centre, skip back and forward 38 pt beside it; button fill white at 0.1 alpha 0.55. A
  scrim behind them: black 0.45 at the top fading to clear by 28%, clear to 66%, black 0.5 at
  the bottom. A 3 pt progress line along the bottom edge, polled every 0.5 s.
- Pinch resizes the window (WebKit's own magnification is turned off while floating).
- Flick it and it lands in the nearest corner on a critically damped spring (omega 15), stepped
  by a display link at up to 120 Hz; thrown against a side it bounces on an underdamped spring
  (omega 16, damping 0.5). Pulled 90 pt past a side it docks there, leaving a 10 pt sliver.
  Tuck and untuck resize over 0.18 s.

How (`Float.swift:1-60, 1155-1260`, `Browser.swift:7306-7380`, `NativePiP.swift`):

- WebKit refuses system picture in picture without a real click, so the page itself is moved:
  an injected stylesheet hides everything (visibility on the body), turns it back on for the one
  video and stretches that video to the viewport, then the tab's whole web view is reparented
  into a borderless, non-activating, floating panel that joins every desktop and full-screen
  space. The player's own machinery keeps running, so the stream never stops or rebuffers.
- During the swap a still of the video's last frame covers the panel until the page has laid
  out at its new size, then fades over 0.12 s, so the video never shows small in a corner.
- Floating on its own happens only on known player sites: a fixed list of about thirty hosts
  (YouTube, Netflix, Prime Video by path, Disney+, Apple TV, Twitch, Vimeo, Arte, TED and
  others) in `Players` (`Float.swift:986-1020`). On the macbook a local test page with a
  playing, audible video did not float when its tab was left; ⇧⌘P lifted it.
- Which video floats on its own: one that has been heard (now, or muted by hand since), is big
  enough to be watched (Safari's main-content rule, about 400 x 300), longer than a loop, and is
  not a hover preview inside a link or thumbnail. ⇧⌘P skips the rules and lifts any playing
  video. WebKit pauses silent videos as their page goes out of sight, so the script remembers
  which ones it paused and plays them again in the window.
- The window closes itself when the video leaves the page, is emptied, or ended more than ten
  seconds ago with nothing after it (one second of grace for a player swapping videos).
- The optional "Picture in picture" style flips a private WebKit preference and asks the page
  for the system window from an app-run script, which WebKit treats as a user gesture.

Lynk: parity Page tools, "Floating video: Alt+Cmd+P lifts the playing (else the largest)
video... into Chromium's picture-in-picture window", and "A page's own picture-in-picture",
lane `blocking-tools`. Chromium allows picture in picture from an app-run script with a user
gesture, so Lynk does not need the reparenting trick for the basic case. What to take: float on
leaving the tab, the space or the app (with the occlusion rule), return on coming back, the
"which video" rule, the "playing in the floating window" placeholder, and the per-site hold. The
reparented page window pays off only for the richer window (whole page alive, own controls,
flick and dock), which Chromium's own window does not offer. nd: moving a live page between
windows exists for tab drag; an always-on-top, all-desktops, non-activating window kind does
not (only `WindowKind::Popup`, `nd-ui/src/window.rs:26-37`); window occlusion state is not
exposed.

## Reading mode

What the user sees: ⇧⌘R replaces the page, in the same tab, with its article: title, a small
uppercase source line, the text and its pictures, nothing else. A three-way switch (sun, moon,
half circle) floats in the top right corner at 0.55 opacity, full on hover, making the article
light, dark or as the window is, kept for the next article. ⇧⌘R again reloads the original page.

Measurable look (`Reader.swift:14-90, 582-621`):

- Column max 38em, centred, padding 72 px top, 24 px sides, 160 px bottom.
- Body 18 px serif (`ui-serif`, Georgia) at 400, line height 1.72; paragraphs 1.35em apart.
- Title 30 px system sans at 600, line height 1.24, letter spacing -0.01em, 8 px under it.
- Source line 12 px uppercase, letter spacing 0.06em, quiet colour, 40 px under it.
- h2 and h3 20 px semibold at 1.3, 2em above; h4 to h6 16 px semibold.
- Images and video full width, radius 6, 1.6em above and below; figure captions 13 px sans in
  the quiet colour; the byline and summary 14 px sans in the aside colour.
- Code 14 px monospace; blocks on the code ground, radius 8, padding 14; inline code radius 4.
- Blockquote: a 2 px rule on the left, aside colour. Tables 15 px sans, 1 px row rules.
- Light: ground #fff, ink #171717, quiet #a3a3a3, aside #555, rule #e8e8e8, code #f5f5f5. Dark:
  #1c1c1c, #e0e0e0, #8a8a8a, #a8a8a8, #333, #2a2a2a, and pictures at brightness 0.92 so they do
  not glare. The dark ground is the window's own ground; the ink a shade softer than the app's.
- The switch: a capsule of three 24 x 22 buttons, 11 pt medium glyphs, the chosen one on the
  wash fill, ground with a hairline, 14 in from the corner; it fades on quick.

How:

- If the site publishes a markdown copy of the page (the page is markdown, names one with
  `<link rel="alternate" type="text/markdown">`, or answers its own address with markdown for an
  `Accept: text/markdown` request), that is drawn instead of guessing, and the source line reads
  "<site> · markdown" (`Reader.swift:458-550`, `Tab.swift:490-540`).
- Otherwise a script scores candidates (`article`, `main`, `[role=main]`, common classes, every
  `div` and `section`) by the letters in their paragraphs (at least two paragraphs, 400
  letters) times one minus the share of their text that is link text. Between a box and one it
  contains, the smaller wins unless the larger scores more than 1.25 times as much, which keeps
  comments out. The first large picture before the text is brought in from up to four boxes out.
- The page's own stylesheets are removed, the body is replaced with the article and a single
  stylesheet, in the page's own document. Leaving reloads rather than restoring the old markup,
  because restored markup has lost every listener.

Lynk: parity Page tools, "Reading mode: Shift+Cmd+R runs Readability over a copy of the page in
a shadow-root overlay, follows light and dark, Space and Page Down scroll it, Esc and a
same-document navigation leave it". Lynk's overlay keeps the page underneath, so Esc leaves
instantly with no reload, which is better than Search's reload. What to take: the type and
colour values above (Lynk's old `src/lib/reader.ts` already used this family, muted at
neutral-500 for contrast), the markdown copy first, the corner switch for light, dark or
window, and dimming pictures in dark. Lane `blocking-tools`. nd: nothing; it is page script and
CSS.

## Smaller surfaces

- Little window: a link from another app opens in a 900 x 640 window with a 34 pt line (site
  name 12.5 medium, head-truncated to keep the domain) and "Open in Search" (⌘O), which moves
  the tab over without reload. Lane `switcher-link-window`.
- Downloads card: a panel by the downloads button that never takes keys, scaling 0.9 to 1 from
  the corner nearest the button. Lane: downloads in `omnibox-panels`.
- Find bar: a capsule top right, 12 in from the top and 14 from the right, shadow 0.10 r 18 y 5,
  outline turns red at 0.35 on no match; matches painted with the CSS Custom Highlight API from
  an isolated script world. Lynk has `crates/lynk/src/find.rs`.
- Site card: a child panel under the address with menu vibrancy and menu metrics (24 rows,
  radius 12), so it looks like a native menu. Lynk: padlock popover (`site_info.rs`).
- Hide anything (⇧⌘H): an outline 2 px at 0.9 ink following the pointer at 0.07 s, Up selects
  the parent; saved selectors are injected at document start so a hidden thing never flashes.
  Lynk has the element hider in `blocking-tools`.
- Walls behind the blank tab: ten scenes drawn in code from a seeded generator, rendered once to
  an image (size rounded up to 64 pt steps, four kept), dark mode at 0.42 of the same colours.
  Lane `layouts-themes`.
- Dark pages ("Dusk"): a 48 px snapshot decides whether a page is already dark; otherwise CSS
  rules are rewritten with relative `oklch()` lightness, falling back to an invert filter. Lynk
  can get this from Chromium's forced dark mode; inspiration section 1 lists it.
- Web Inspector: WebKit's own, docked beside the page, detachable. An optional Chrome DevTools
  package docks at the bottom or a side. Lynk's DevTools docking box is in `blocking-tools`.
- Update notice: a row at the foot of the column, or a pill at the end of the strip, never a
  dialog over the page.

## Ranked: most look and feel for the least code

| # | Item | Size of change | Lane | nd needs |
| --- | --- | --- | --- | --- |
| 1 | Quieter fills: selected row about 0.063, hover 0.035 (black over light, white at 0.073 / 0.045 over dark), three title inks (full, 0.7 hover, muted), hover colour not animated | Constants in `sidebar.rs` | shell now, `sidebar-polish` | nothing |
| 2 | Three motion tokens (glide 0.34 / 0.82, settle 0.30 / 0.86, quick 0.14 s ease-out) and one reduced-motion gate, used by every lane | One module of constants | shell | `MotionTokens` exists (spring 0.35 / 0.86); match or override |
| 3 | The selected-row pill glides between rows and pins, never across spaces | One indicator element | `sidebar-polish` | prove `hero_from` between sibling rows |
| 4 | Title fades under the hover cross instead of truncating earlier; cross 15 pt with a 30 pt hit area | Row layout | `sidebar-polish` | a gradient mask on an element |
| 5 | One floating recipe: ground, 1 pt hairline, continuous radius 12 to 16, soft shadow, dim graded by modality | Shared style helper | command bar, every overlay | nothing |
| 6 | Command bar: list anchored below the field so it never moves, neutral wash selection not accent, hover separate from keyboard selection, ground at 0.74 over the page, enter at scale 0.97 plus opacity | Command bar view | command bar (running) | nothing |
| 7 | Page card: one device pixel hairline, no soft shadow, 8 gap, radius 10, offsets snapped to pixels; animate the clip, not the layout | Window layout | `sidebar-polish`, Linux bring-up | radius clip of a web view on WK, Linux and Windows |
| 8 | Edge reveal at Search's speed: 6 pt zone, 65 ms grace, spring 0.14 / 0.86 out and 0.13 / 1.0 in, shadow cast from a shape behind the rows | Timing values | `sidebar-polish` | spring options on `edge_reveal` |
| 9 | Settings as one index: search every word, jump and highlight 1.4 s, dependent rows dimmed with their reason, sliding rail pill | Builds on `settings_registry.rs` | settings tab (running), `settings-design` | nothing for rows; thumbnail picker later |
| 10 | Drag with the carried row 1:1 and neighbours on settle, ghost slot when crossing pins and tabs, haptic tick | List drag | `pins-profiles` (running), `sidebar-organise` | spring displacement and ghost slot in `source_list` |

Next after these: the visual ⌃Tab switcher (needs CEF snapshots in nd), peek as an off-row tab,
split with snap and haptic, the space swipe and tint, balanced pin rows, the keep line's Clear,
back-to-pinned on the favicon, ⌘-held digits, the compositor-only loading ring.

## What nd would have to provide

| Need | For |
| --- | --- |
| Linear gradient mask on an element (title fade) and gradient fills with grain (space tint) | rows, spaces |
| `hero_from` proven to glide between sibling rows on AppKit and GPUI | selected pill |
| Spring options on `edge_reveal` | fold |
| Spring displacement, 1:1 carried row and ghost slots in `source_list` | drag |
| Balanced row filling in `tile_grid` | pins |
| Page snapshot from CEF and WKWebView | switcher, sleeping tabs |
| A modifiers-changed event in nd-ui | ⌃Tab release, ⌘-held digits |
| Corner radius clip of a web view on WK, Linux and Windows | page card |
| An always-on-top, non-activating window kind | floating video, launcher, downloads card |

## Could not be established

- Motion was read from the source only. The macbook has Reduce Motion on, under which Search
  turns every animation off, so no timing above was seen or measured on screen.
- Peek, space swipes, drag and the space tints were not run: they need a setting changed
  through a UI whose controls are not in the accessibility tree, or a trackpad.
- The roadmap page loads its list with script and showed nothing to a plain fetch; what is
  being built next comes from `ROADMAP.md` and the changelog instead.
- No Linux or Windows build exists, so nothing here says how Search's look survives off macOS;
  each item's nd note says what Lynk would need there.
