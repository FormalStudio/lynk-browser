# Inspiration: bookmarked posts

Seven X posts the owner bookmarked on 2026-11-23 and 2026-11-24 for features, design and ideas. These are
features the owner wants built, not optional reading: each is scheduled as a lane in wave 5 of
[delivery-plan.md](delivery-plan.md), and lanes that touch the same surface earlier (settings, the
sidebar, pins) follow the design here from the start. Each item names where it would live (Lynk, nd, or both) and what it needs to
work on macOS, Linux and Windows. Read from the posts' images and from one frame per second of
each video; the authors' profile pages could not be loaded (X rate limit), so product names come
from what is visible on screen.

## Sources

| # | Post | Product shown | What it is |
| --- | --- | --- | --- |
| 1 | [officecommun, 2026-11-23](https://x.com/officecommun/status/2124822257029521409) | "Search", a Mac browser | 28 s tour of its settings |
| 2 | [MrBye32, 2026-11-22](https://x.com/MrBye32/status/2124592530436071927) | "Rake" (`1bye/rake`), GPUI with GPUI Motion | 21 s of sidebar drag, pin and separators |
| 3 | [mcign69, 2026-11-22](https://x.com/mcign69/status/2124615297151676661) | "Ask", Rust on Chromium, slate theme | 4 stills, horizontal tabs |
| 4 | [mcign69, 2026-11-22](https://x.com/mcign69/status/2124588705683832905) | "Ask", plum theme | 4 stills, vertical sidebar |
| 5 | [oleksiibe, 2026-11-05](https://x.com/oleksiibe/status/2118418649816220116) | unnamed CEF browser | 19 s of a visual tab switcher and a small link window |
| 6 | [oleksiibe, 2026-10-20](https://x.com/oleksiibe/status/2112601852399280358) | same browser | 17 s of trackpad gestures with force click |
| 7 | [mcign69, 2026-11-24](https://x.com/mcign69/status/2125047872684982713) | "Ask", plum theme | 3 stills: split tabs, typed folders, folder popover |

## 1. Settings that stay small (post 1)

The post's point: the browser looks minimal, and everything extra is a setting that is off by
default.

What is on screen:

- Settings is a tab (⌘,), listed in the sidebar like any page, with its own narrow nav column:
  General, Appearance, Themes, Tabs, Browsing, Spaces & Pins, then Passwords, Privacy, Extensions,
  Keyboard & Mouse, then About.
- Every setting is one row: icon, title, an info dot, one plain sentence saying what it does, the
  control on the right. Sections have small caps headers. No nested pages.
- "Find a setting" (⌘F) filters across all sections. Results read "Settings matching "dark"",
  grouped under a breadcrumb (`Appearance › Light and dark →`) that jumps to the section. Nav
  entries without a match dim, the others show a match count.
- Appearance is three thumbnails (Light, Dark, Auto) and applies while the page is open. The
  window recolours in place.
- Tab layout is four thumbnails: Top, Left, Right, Icons only. Icons only shrinks the sidebar to
  a strip of favicons, stacks the traffic lights vertically and keeps the foot buttons. Two
  related toggles: "⌘S makes the column icons only" and "Narrow to icons" (dragging the sidebar
  under 200 pt turns it into the strip).
- Rows that depend on another setting stay visible but disabled, with the reason as their
  description ("Only while Search isn't set to Light").

Settings worth taking, by section:

- General: default browser, which mail app email links open in, import from another browser,
  sync through a folder you choose, confirm before quitting. Downloads: save location, ask per
  file, always show the downloads button, open Downloads in a tab or a panel.
- Tabs: new tab position, links opened in front go beside their page, leave a new tab when the
  last one closes, closing a tab returns to the one before, swipe back closes a tab a link
  opened, ⌘Z reopens a closed tab, Ctrl+Tab goes to a link opened behind, Ctrl+Tab leaves out
  pinned tabs, load background tabs only when shown, sized pop-ups open as small windows.
  Memory: sleep unused tabs (Never, Later, Normal, Sooner), dim sleeping tabs.
- Spaces & Pins: "Set up like Arc" as one button, import Arc's spaces and pins, spaces on or
  off, ⌘1 to ⌘9 switch spaces, one foot button per space, profiles, send links for chosen sites
  to a space, ask which space a link from another app opens in, pins keep their site, pinned
  rows, ⌘-click on a pin opens a new tab, new tab as a floating field.
- Keyboard & Mouse: peek trigger (a dropdown, Shift-click shown), peek stays with its tab,
  Alt-click opens a link in split view, middle-button scroll, smooth wheel scrolling, bounce at
  the ends, hold a back swipe to pick from history, a global key that raises the address field
  over any app, open that result in a small window, and a searchable shortcut editor.
- Extensions: add by Chrome Web Store link or id, bring them over from another browser, load
  unpacked.
- Appearance extras: darken pages that have no dark look, DevTools colours follow or stay dark,
  reading mode follows light and dark.

Where it lives: Lynk (the settings page and its registry) and nd (thumbnail picker, segmented
control, searchable form rows as components).

Multiplatform: shortcuts shown as ⌘ need the Ctrl form on Linux and Windows from one table. The
traffic-light stack is macOS only; on Linux and Windows the icon strip needs a decision on where
window controls go. Drop the Mac-only rows (Handoff, iCloud, Safari app extensions) rather than
show them disabled.

## 2. Sidebar drag, drop to pin, named separators (post 2)

- Dragging a tab row lifts it (slightly larger, shadowed) and the other rows spring out of the
  way as it passes. The rows settle with a spring, not a linear slide.
- Dragging a row up into the pinned tiles: the divider under the tiles shows "Drop to pin", the
  row morphs toward a tile and a slot outlines where it will land.
- Tab context menu: Reload Tab, Duplicate Tab, Pin Tab, Mute Tab, Copy Link, Move to root, Add
  separator below.
- A separator is a thin rule with an optional name. "Add separator below" inserts it with a text
  field already focused; Enter commits ("Hey" in the video). It then drags like a tab row, and
  tabs reorder across it.
- The divider between pins and tabs carries two buttons: new folder and new tab.
- The author's own doubt and idea: separators are lighter than folders, and grouping could be
  automatic ("AI grouping").

### Generated separators (wanted, from the author's idea)

Not shown in the video; the owner wants it. One action sorts the loose tabs into groups and
writes a named separator above each group, using the same separator rows as the manual ones, so
the result can be renamed, dragged or deleted by hand afterwards.

- Input is only what the sidebar already knows: each tab's title, host and the tab it was
  opened from. No page content.
- It runs on demand ("Tidy" in post 4 is the natural button for it), never on its own, and one
  undo puts the list back.
- Pinned tabs and tabs already inside a folder or project are left alone.
- Models: on Apple devices, the on-device models Apple ships with the system (the Foundation
  Models framework), so nothing is downloaded and nothing leaves the machine. On Linux and
  Windows, an optional small local model the user chooses to download; without it the action
  is absent or falls back to grouping by site, which needs no model.
- The model's only job is to return group names and which tab goes where. Lynk validates the
  answer (every tab exactly once, no invented tabs) before touching the list.

Where it lives: Lynk for the action, the fallback and the validation. nd for one "small text
model" interface with two backends (Apple's system model, a bundled local runtime), since
other nd apps will want the same thing.

Where it lives: Lynk for separators, the menu items and drop to pin. nd for the list: a
reorderable list whose rows displace with springs, a drag preview that can morph between two
row shapes, and inline rename in a row.

Multiplatform: nothing platform specific. Honour reduced motion with a crossfade instead of
displacement. Separators must be reachable from the keyboard (menu key, then rename).

## 3. "Ask": two layouts, one design language (posts 3 and 4)

Horizontal layout (slate theme):

- The title bar holds, left to right: traffic lights, a pill with the space name, pinned sites
  as bare favicons, the open tabs with titles, a plus.
- The space pill opens the app menu: an amber "Restart to update 1.4.2" row first, then Spaces
  (Work ^1, Personal ^2, Edit space, New space), then History, Extensions, Downloads,
  Bookmarks (submenu), Settings, and a red Quit. Shortcuts are right aligned. When an update is
  ready the pill itself shows a download glyph.
- New tab page: one wide field ("Search or type a URL") with a microphone and a two-way switch,
  Search or Ask, at its right end. A dithered sphere above it. "Handle" at the top edge and
  "Chats" at the bottom edge, each with a chevron, read as panels you pull in. A palette button
  in the bottom right corner opens themes.
- An amber "1 need you" pill at the top right: background agents waiting on the user.

Vertical layout (plum theme):

- Sidebar top to bottom: traffic lights, sidebar toggle, back, forward, reload; the address
  field; pinned sites as a 4 by 2 grid of tiles; a collapsible "Chats" list with times and a
  compose button; a rule with a "Tidy" button; "New tab"; loose tabs; folders.
- Folders are cards with a tinted fill. Open, they list their tabs indented. Closed, the row
  shows up to three favicons and a count ("Reading list", 3 icons, "3").
- Projects: the first card is titled "Project · Ask", set apart from the plain folders "Daily"
  and "Reading list" by the "Project ·" prefix. It holds the tab in front ("New tab") and a
  Notion page ("Work overview"). The still shows nothing more of how a project differs from a
  folder, so this is the reading to build from: a project is a named working set in the
  sidebar that owns its tabs, and new tabs opened while one of its tabs is in front join it.
  Wanted in Lynk. Open questions for the owner: whether a project also keeps its own pins and
  chats, and whether closing a project sleeps or archives its tabs. Post 7 shows the same
  card filled in, see section 6.
- Row trailing indicators: a dot for unread, a speaker for audio, a close button on hover.
- Above the foot: "1 needs you, 5 agents" and "Update ready" with a Restart button, as two
  stacked rows that only appear when relevant.
- Foot: downloads (with a dot while something is new), the space name with a chevron, the
  profile.
- The page is an inset card with rounded corners, and the window colour shows around it.

Look: each theme is one hue pair applied to the window, sidebar, cards and menus. A dithered
halftone texture drifts across empty surfaces. Type is a geometric sans at one weight for most
rows, two for headers.

Where it lives: Lynk for the top-tabs layout, the app menu on the space pill, update rows,
folder cards, "Tidy" (clear or archive loose tabs), themes. nd for theme tokens that tint every
surface from one pair, and a title bar that accepts arbitrary controls beside the window
buttons.

Multiplatform: controls beside the window buttons need the Windows caption buttons on the
right and client-side decorations on Wayland. Menus with a coloured row and a destructive row
have to be nd menus, not the native menu, to look the same everywhere. The Ask mode, Chats and
agents are a product direction, not a layout detail: recorded here as seen, out of scope until
the owner says otherwise.

## 4. Visual tab switcher and a small link window (post 5)

- Holding the switcher key blurs the page and lays a row of five cards across the middle: a
  page thumbnail, the favicon as a badge on its lower left corner, the title underneath. The
  selected card has a ring and a brighter title.
- Order is most recently used, and it reorders after each switch. A blank tab shows a plain
  card with a globe.
- Letting go crossfades the cards out while the chosen page sharpens. Cards can also be
  clicked.
- The window has no visible chrome while browsing: the sidebar is hidden and slides in as a
  translucent panel when the pointer reaches the left edge. The page sits in a thick, light,
  rounded frame.
- A link from another app opens in a small separate window showing only the site name, a
  loading mark, site settings and an "Open in Personal" button (⌘O) that moves it into that
  space as a tab.

Where it lives: Lynk for the switcher (it replaces the text list for Ctrl+Tab; ⌘K stays the
text switcher in `docs/omnibox.md`) and the small window (already listed in wave 5 of
`docs/delivery-plan.md`). nd for page snapshots, a backdrop blur over a live web view, and
detecting that a modifier is still held.

Multiplatform: thumbnails must come from the engine (CEF), not a screen capture, so they work
on Wayland. Blur over a web view is cheap on macOS and needs checking on Linux and Windows; a
dimmed scrim is the fallback. Modifier-held state has to come from the key-up event, which
works on all three.

## 5. Peek by pressing harder (post 6)

The video shows a hand on a trackpad next to the window.

- A force click on a link opens it as a peek: a card over the dimmed tab, narrower than the
  page, with the domain in small type on its top edge and two buttons outside its right edge,
  close and expand.
- The peek scrolls and is fully usable.
- A further gesture grows the card until it fills the page area and it becomes a real tab. Its
  row slides into the sidebar directly under the tab it came from. The reverse gesture shrinks
  it back toward the link and closes it.
- Peeks nest: a link pressed inside a tab that came from a peek opens another one.
- The exact gestures after the force click (pinch, deeper press or swipe) cannot be read from
  the footage. The post only says "it's all gestures (feat. force click)".

Where it lives: Lynk for peek (wave 5 lists it) and the rule that a promoted peek lands under
its parent tab. nd for pressure and gesture input, and an interruptible card-to-page
transition that follows the fingers.

Multiplatform: force click exists only on Mac trackpads. Every gesture needs a second trigger
that works everywhere: Shift-click (as in post 1) or a long middle-click to open, Escape or a
click outside to close, a key and the expand button to promote. Linux touchpad pinch and swipe
arrive through the compositor's gesture protocol and Windows through precision touchpad
events; treat both as optional and never as the only way.

## 6. Split tabs, typed folders, a folder popover (post 7)

Three stills that join into one wide window. The post's pitch: "what if all the things loved
about Arc were in rust? split tabs, sidebar, peek and more."

Split tabs:

- The page area holds two page cards side by side, each with its own rounded corners, and a
  short vertical grabber centred in the gap between them for resizing. Here the left card is
  the new tab page and the right one is a YouTube video, at roughly 60 to 40.
- In the sidebar the pair is one row split into two halves, each half with its favicon and
  title ("New tab" and "Never Gonna..."). The half in front has the brighter fill, so the row
  shows both which split is open and which side has focus.
- A split can be kept: inside the "Daily" folder, "Inbox (12)" and "Calendar" sit together as
  one two-half row, alongside a normal single row for Spotify.
- Titles in a half truncate much earlier than in a full row; the favicon carries the half.

Typed folders:

- Each folder card has its own icon and tint: a code glyph for "Project · Ask", a sun for
  "Daily", an open book for "Reading list" (warmer fill than the other two).
- "Project · Ask" now lists four tabs from four tools about one piece of work: a Figma file
  ("Split view"), a GitHub pull request ("Split tabs · PR #2184"), a Linear issue ("ASK-412
  Resize handle") and a Notion page ("Work overview"). This supports the reading in section 3:
  a project is a working set gathered around one task, across sites.
- A loose tab above the folders ("Sony vs Bose, RTINGS.com") matches a chat of the same
  subject, so pages an agent opened appear as ordinary tabs.

Folder popover:

- The collapsed "Reading list" row (three favicons and a count) opens a popover beside the
  sidebar, anchored to the row: a "Search tabs" field already focused, then the folder's tabs
  as rows with favicon, title and the host right aligned in a dimmer colour, the first row
  selected.
- Its footer is one command, "Open as split", with the shortcut ⌘↵. So a folder can be
  browsed, filtered and opened without expanding it in the sidebar, and Enter with a modifier
  puts the chosen tab beside the current one.

Other details:

- Chats carry a status mark: three use a dithered orb in different tints and "Morning brief"
  uses a plain speech bubble. The tints are not labelled; they read as agent states.
- The update row from post 4 is absent, which confirms those foot rows appear only when
  relevant.

Where it lives: Lynk for the split model (a row that owns two tabs, kept inside folders, focus
per half), folder icons and tints, the folder popover and "Open as split". Split view and
folders are both in wave 5. nd for a list row with two independently clickable and draggable
halves, a resizable two-pane container holding two web views, and a popover anchored to a
sidebar row that contains a filtered list.

Multiplatform: nothing platform specific in the layout. "Open as split" needs Ctrl+Enter on
Linux and Windows. Two live web views in one window is the part to check on every engine
path, Chrome style under XWayland included: focus, the resize drag crossing a web view, and
which page the address field and shortcuts act on. Decide early what a split does in the
icons-only sidebar (two stacked favicons in one cell is the obvious form).

## What recurs across the posts

- A hidden or icon-only sidebar with the page as an inset rounded card (1, 3, 4, 5, 6, 7).
- Pins as square tiles above the tabs, with a labelled divider below them (2, 4, 6).
- Motion that tracks the hand: spring displacement, morphing drags, interruptible transitions
  (2, 5, 6).
- One line of plain language per control, and off by default (1).
- Update and attention states as rows in the chrome, not dialogs (3, 4).

## What nd would have to provide

| Need | For |
| --- | --- |
| Reorderable list with spring displacement and morphing drag previews | 2 |
| Inline rename inside a list row | 2 |
| Small text model interface: Apple's system model, optional local model elsewhere | 2 |
| Settings row, thumbnail picker, segmented control, form search | 1 |
| Theme tokens derived from a hue pair, optional surface texture | 3 |
| Custom controls in the title bar on every platform | 3 |
| Custom-drawn menus with accent and destructive rows | 3 |
| List row with two independent halves | 6 |
| Resizable two-pane container for two web views | 6 |
| Popover anchored to a list row, with a filter field and list | 6 |
| Page snapshots from the engine | 4 |
| Backdrop blur or scrim over a live web view | 4, 5 |
| Pressure, pinch and swipe input with a keyboard and mouse equivalent | 5 |
| Gesture-driven, interruptible transitions | 5 |

## Already on the plan

Wave 5 of `docs/delivery-plan.md` covers spaces, folders, split view, auto-archive, peek and
the small window for links from other apps; it was declined on 2026-09-29 and stays out until
the owner says yes. This document adds detail to those and adds new items: named separators, generated
separators, projects in the sidebar,
drop to pin, the icon-only sidebar, the settings design, the visual switcher, top tabs, themes
and gesture input.
