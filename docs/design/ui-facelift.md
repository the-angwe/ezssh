# ezSsh UI facelift

Rev 2, 2026-11-03. Status: **implemented** (phases F0–F5 below, on the
`native/phase-8-cutover` tree, uncommitted). Rev 1 was the proposal; the
decisions D1–D4 were all taken as recommended. Mockups:
https://claude.ai/artifact/XTD1YhDawuhwoYsRswxkzh (current shell, proposed
dark + light shell, session dialog, settings, component sheet with tokens).
§11 lists where the implementation departs from the proposal.

## 1. Goal and constraints

Make the chrome look contemporary and make every screen use the same
vocabulary, without touching what the app *is*: the MobaXterm layout
(sessions sidebar, tabbed terminal area, file pane docked left of the
active tab, status bar) stays, as do all flows, shortcuts, the data
formats and the AccessKit labels the headless tests find widgets by.

egui 0.37 limits: no backdrop blur, no window translucency, one shadow
per frame, integer corner radii. Everything proposed below is flat
painting the views already do by hand, so nothing needs a new crate.

## 2. Audit: what is inconsistent today

Found by reading `style.rs`, `widgets.rs`, `form.rs` and the views, and
by looking at the `EZSSH_UI_SHOTS` renders (27 of 28 UI tests pass on the
`native/phase-8-cutover` tree; `sftp::file_pane_end_to_end` fails on a
folder-job assertion unrelated to the UI).

| Area | Today | Problem |
|---|---|---|
| Corner radius | 1 (rails), 2 (buttons, inputs, badges, toasts), 3 (dialogs, menus, cascade frames), 4 (section cards, option cards) | four ad hoc values; 2–3 px reads as 2015 web admin |
| Separators | 1 px `border` lines around every region: sidebar, tab strip, pane headers, status bar, tabs between each other | boxed look; the eye reads lines before content |
| Type | rows, menus, toasts, status bar at `XS` = 11 px; uppercase 10 px captions in bordered section cards | too small for primary content; cards-inside-cards |
| Control heights | primary button 26, ghost 28/30, input 30, icon buttons 20 / 24, menu rows 24, sidebar rows 28, nav rows 30 / 32 | nothing lines up in a footer or a toolbar |
| Dialog chrome | `DialogHead::None / Strip (42) / Full (56)`, plus hand-painted headers in `settings.rs` (40) and `session_dialog.rs` (56); two side-rail implementations (`form::side_tab`, settings nav) | same thing drawn four ways |
| Selection | left 2–3 px accent bar + fill (settings nav, side tabs, sidebar rows) | the “left border” idiom, applied inconsistently |
| Tab strip | 180–260 px tabs, vertical lines between, bottom accent line, pane-toggle buttons inside every tab, six view-mode icons at the right | browser-of-2012 look; toggles crowd the tab |
| Status bar | 24 px, 11 px **mono** text; theme / settings / updates / lock as 20 px icon buttons at the far right | app actions hidden where nobody looks; mono for prose |
| Sidebar | no filter; trash icon on folder hover; folder icons in accent | destructive hover affordance; accent used as decoration |
| Primary button | `accent` fill with white text: ~3.5 : 1 on dark | fails 4.5 : 1 for 12 px text |
| Toasts | left colour bar, 2 px radius, 11 px | same “left border” idiom |

## 3. Direction: quiet layers

Fluent-adjacent (the app is Windows-first and already uses Segoe UI
Variable), without copying Fluent's chrome:

- **Tonal layers, not lines.** Sidebar, panes, strip and status bar sit
  on `surface`; the terminal area is the recessed `bg`. Hairlines stay
  only where a palette-driven terminal meets chrome, and they are `fg`
  at 8 % alpha so one token works on every surface.
- **One radius scale.** 4 checkbox, 6 controls / rows / menu items,
  8 cards / menus / toasts, 12 dialogs, pill for badges and chips.
- **Pill selection.** Selected rows and nav items are a rounded
  `accent-soft` fill inset 6 px; the accent bar disappears everywhere.
- **Accent is for state.** Selection, focus, the active pane toggle,
  progress. Folder icons, section captions and decorative strips go
  back to `muted`.
- **13 px is the floor** for anything the user reads; 11 px only for
  uppercase captions and badges.
- **Dark-theme primary = light accent with dark text** (Fluent's rule):
  `#6CA6F5` on `#0F1217` is 9 : 1. Light theme keeps white on `#1E6ED0`
  (5.6 : 1).

## 4. Tokens (`style.rs`)

| token | dark | light | note |
|---|---|---|---|
| bg | #181A1E | #F3F4F7 | window ground, terminal surround |
| surface | #1F2126 | #FFFFFF | sidebar, panes, bars, dialogs, menus |
| surface2 | #2A2D34 | #EEF0F4 | inputs, hover |
| surface3 | #343842 | #E3E6EC | **new**: pressed, switch track, progress track |
| border | fg @ 8 % | fg @ 9 % | hairlines (was an opaque colour) |
| border_strong | fg @ 16 % | fg @ 18 % | input borders, secondary buttons, dialog edge |
| fg | #E6E8ED | #1A1D23 | |
| fg2 | #B4B9C4 | #4B5261 | **new**: secondary text, row labels at rest |
| muted | #8B919E | #697080 | captions, icons at rest (≥ 4.5 : 1 on surface) |
| accent | #6CA6F5 | #1E6ED0 | |
| accent_soft | accent @ 14 % | accent @ 12 % | **new**: selection pills, focus ring |
| on_accent | #0F1217 | #FFFFFF | **new**: text on the primary button |
| success / warning / danger | #4FB37E / #E0B04A / #EF6B6B | #1F8A50 / #A86E12 / #C23A42 | |

Scales (new `pub mod radius` / `space`, and the size constants):

- radius: `SM 4`, `MD 6`, `LG 8`, `XL 12`, `PILL 999`
- space: 4 / 8 / 12 / 16 / 20 / 24; dialog padding 24, pane padding 8,
  row inset 6
- heights: `ROW_H 28`, `NAV_ROW_H 32`, `CONTROL_H 32` (dialogs),
  `CONTROL_H_SM 28` (panes, toolbars), `ICON_BTN 28`, `TAB_BAR_H 36`,
  `PANE_HEAD_H 36`, `STATUS_BAR_H 26`
- type (`size`): `XS 11` captions / badges, `SM 12` secondary, `BASE 13`
  body, `MD 14` section titles, `LG 16` dialog titles, `XL 20` unlock;
  mono 12 for paths, fingerprints, `user@host`
- shadows: dialog `0 16 48 @ 50 %`, menu / toast `0 8 24 @ 35–40 %`
- egui visuals: `window_corner_radius 12`, `menu_corner_radius 8`,
  widget radii 6, `interact_size 28`, `button_padding (12, 6)`,
  `item_spacing (8, 6)`

## 5. Components (`widgets.rs`, `form.rs`)

- **Button**: one builder with `Variant::{Primary, Secondary, Ghost,
  Danger}` and `Size::{Md 32, Sm 28}`; replaces `primary`,
  `primary_with_icon`, `primary_full`, `Ghost`. Secondary = `surface2`
  fill + `border_strong`. Focus ring 2 px accent outside, radius + 2.
- **IconButton**: 28 × 28, 16 px glyph (14 in dense rows), radius 6,
  `on` state = `accent_soft` fill + accent glyph. Replaces the 20 / 24
  variants.
- **Input / Secret / Textarea / Select**: 32 high, radius 6, `surface2`,
  `border_strong`; focus = accent border + 3 px `accent_soft` ring;
  error = danger border + ring and a 12 px message under the field.
  Number fields get a stacked ▲▼ stepper (replaces `DragValue`).
- **Switch** (new): 36 × 20 pill; used for every boolean setting row.
  Checkbox stays for in-form options (16 px, radius 4).
- **Segmented** (new): pill group, 26 px items, selected item lifted on
  `surface`; replaces 2–4 option radios and the view-mode icon row.
- **NavRail** (new, shared): 176 px, 32 px rows, pill selection;
  used by the session dialog and settings (drops `form::side_tab` and
  the hand-painted settings nav).
- **Dialog**: one chrome. `XL` radius, `border_strong` edge, 56 px
  header (optional 28 px icon tile, 16 px title, 12 px mono subtitle,
  ✕ IconButton), body padding 24, 60 px footer with a hairline and
  right-aligned buttons, keyboard hints on the left in 12 px muted
  `kbd`. `DialogHead::Strip` and the strip fill go away; the compact
  confirm uses the same card without a header (icon tile + title +
  body + footer). `settings.rs` and `session_dialog.rs` use it.
- **Section** (`form::section`): no card. 14 px semibold title, 12 px
  muted description, 20 px below, hairline between sections.
- **Row** helper (new): 28 px pill row with hover / selected states,
  used by the sidebar, the file list, forwards rows and the minimised
  strip, so all four stop drawing their own.
- **Menu**: radius 8, 6 px padding, 28 px items radius 5, 13 px text,
  optional leading icon and right-aligned shortcut in 11 px muted;
  danger items in `danger`.
- **Toast**: radius 8, no colour bar; kind shown by a tinted 24 px
  circle icon; 13 px title, 12 px body; `border_strong` edge.
- **Chip** (new): 18 px pill for the status bar (cwd, X server, update)
  and counts; replaces the folder badge rectangle.
- **EmptyState**: unchanged apart from sizes (icon 32, title 14).

## 6. Screens

**Shell** (`shell.rs`, `sidebar.rs`, `tabstrip.rs`, `statusbar.rs`,
`mdi.rs`)

- Sidebar: top row = filter field (`Ctrl+K`, filters the tree as you
  type, keeps folders of matches open) + `＋` menu (New session / New
  folder / Import…) + collapse. Tree rows 13 px, `fg2` at rest, pill
  hover / selection, 18 px colour tiles stay (they are the one
  identity device), folder icon `muted`, count as a Chip, a 6 px
  status dot at the right of sessions with an open tab. Folder delete
  moves to the context menu only. Footer: lock state label, lock,
  theme and settings IconButtons (moved from the status bar). Collapsed
  rail 32 px with a 16 px glyph.
- Tab strip 36 px: tabs are 30 px cards with radius 8 on top; the
  active tab takes `bg` and merges into the terminal with no bottom
  line; session colour is a 2 px top edge; status dot left, close ✕
  on hover and on the active tab; min 120 / max 200 px. Right end:
  Segmented pane toggles (file pane, forwards) acting on the active
  tab, then a `Layout ▾` ghost button whose menu lists the six view
  modes with their icons (the six-icon row goes; decision D1).
- Panes: 36 px header, breadcrumb as 12 px mono links, 28 px
  IconButtons; column captions 11 px uppercase; 26 px rows; transfer
  strip as a card (radius 8, 4 px progress bar on `surface3`). The
  forwards pane uses the same header and rows.
- Tiles: 4 px `bg` gutters instead of 1 px `border_strong` lines; the
  active cell ring stays 1 px accent. Cascade frames radius 8, 28 px
  chrome, shadow `0 8 24`.
- Status bar 26 px, UI font 12 px: dot + `user@host:port`, Chips for
  SFTP cwd (mono), X server state and an update when one is known;
  version in 11 px muted. No buttons.
- Unlock card: 360 wide, radius 12, 48 px icon tile, 20 px title,
  36 px input and full-width primary.

**Dialogs**: session, settings, credentials, forwards, backup / restore
/ sync / import, vault, update, X server, host key, paste and close
confirms all go through the one Dialog; the session dialog's type and
auth pickers become option cards (radius 8) and a Segmented; settings
boolean rows become Switches and cursor / right-click rows Segmented.

## 7. Behaviour bundled with the facelift (small, each optional)

1. Sidebar filter field (`Ctrl+K`).
2. `＋` menu in the sidebar replaces three icon buttons.
3. Settings, theme and lock move from the status bar to the sidebar
   footer; the status bar only shows state.
4. `Layout ▾` menu replaces the six view-mode icons (D1).
5. Folder delete only from the context menu (no hover trash).
6. Pane toggles leave the tabs for a strip-level Segmented.
7. Connected-session dot in the sidebar.

## 8. Implementation plan

| Phase | Scope | Files | Size |
|---|---|---|---|
| F0 | tokens, scales, egui visuals; `border` as alpha; `on_accent`; snapshot the 110 UI-test renders before and after | `style.rs`, `ui_font.rs` | ½ day |
| F1 | Button / IconButton / Input / Select / Switch / Segmented / Chip / Row / Menu / Toast / Dialog chrome; delete `Ghost`, `primary_*`, `DialogHead::Strip`, `side_tab` | `widgets.rs`, `form.rs`, `toasts.rs` | 2 days |
| F2 | shell: sidebar (filter, menu, footer, rows), tab strip, status bar, collapsed rail, tiles / cascade | `sidebar.rs`, `tabstrip.rs`, `statusbar.rs`, `shell.rs`, `mdi.rs`, `app.rs` (shortcut) | 2 days |
| F3 | dialogs on the shared chrome + NavRail; sections without cards; option cards; Switches in settings | `session_dialog.rs`, `settings.rs`, `credentials.rs`, `forwards.rs`, `vault_dialogs.rs`, `backup/restore/sync/import/update/xserver.rs`, `dialogs.rs`, `unlock.rs` | 2–3 days |
| F4 | panes: SFTP + forwards headers, rows, transfer card; appearance dialog | `sftp.rs`, `forwards.rs`, `appearance.rs` | 1 day |
| F5 | light-theme pass over every render, contrast check of every text / background pair, README screenshots | — | ½ day |

Test impact: the kittest suites find widgets by AccessKit label and
role, so relabelling nothing keeps them green; tests that click by
rectangle (`tabstrip`, `sidebar` drag tests) need their coordinates
re-derived from the new constants. `fit_widths` / `drop_target` unit
tests change only their `TAB_MIN_W` expectations.

## 9. Decisions to take before F2

- **D1** `Layout ▾` menu vs the six icons. Recommended: the menu (the
  icons are cryptic and the mode changes rarely); keep `Ctrl+Shift+1…6`.
- **D2** dark-theme primary: light accent with dark text (recommended,
  9 : 1) vs white on blue (current, 3.5 : 1).
- **D3** app actions in the sidebar footer (recommended) vs keeping them
  in the status bar.
- **D4** sidebar filter in F2 (recommended; it is the one addition
  users of 100+ sessions ask for first) or deferred.

## 10. Acceptance

- every text / background pair ≥ 4.5 : 1 in both themes (11 px captions
  included);
- one dialog chrome, one row style, one button family across the 110
  rendered screens;
- `cargo test --workspace` green, `cargo clippy -- -D warnings` clean;
- the README screenshots regenerated from the renders.

## 11. Implementation notes (rev 2)

What landed, and where it differs from §4–§7:

- **Tokens** (`style.rs`): as in §4, with `border` / `border_strong` as
  `fg` alpha and the new `surface3`, `fg2`, `accent_soft`, `on_accent`.
  `style::tests::text_contrast_floor` checks the pairs. `muted` meets
  4.5 : 1 on `bg` and `surface` only; on the raised fills (`surface2`,
  `surface3`) text is `fg` / `fg2`, which the test enforces instead.
- **Widgets** (`widgets.rs`): `Button` builder (`Variant`, `Size::Lg /
  Md / Sm`, `.tint`, `.chevron`), `icon_btn` 28 / 16, `switch`
  (AccessKit `Role::Switch`), `segmented` + `toggle_group`, `chip` /
  `chip_button`, `icon_tile`, `row_fill`, `nav_rail`, `Menu` with
  icons and check marks, one `Dialog` chrome (`dialog_frame`,
  `dialog_header`, `dialog_footer` are public so settings, the session
  editor and the auth-fix overlay share it). `Ghost`, `primary*`,
  `DialogHead::Strip`, `form::side_tab` and `egui::DragValue` are gone;
  `form::number` is the stepper field. The header ✕ is labelled "Close
  dialog" so footer "Close" buttons stay unambiguous for the tests.
- **Shell**: sidebar filter (`Ctrl+K`, Escape clears; folders with a
  match open while filtering), the ＋ menu is labelled "New", the footer
  holds updates / lock / theme / settings and the collapsed rail repeats
  them; tab cards 120–200 px with the Layout ▾ menu and the pane toggles
  at the strip's right; status bar with chips (the update chip opens the
  update dialog); tiles with 4 px gutters; cascade frames radius 8.
- **Dialogs**: every dialog is on the shared chrome. Settings uses
  switches, segmented pickers (when the labels fit the 260 px control
  column, else a select) and steppers; the session editor's header is a
  kind icon on a tile in the session colour.
- **Panes**: 36 px headers, 26 px rows with the pill hover / selection,
  transfer cards, forwards rows without hairlines.
- **Tests**: `ui_tests` pick view modes through `pick_layout` and the
  sidebar menu through `sidebar_new`; the settings switch is found by
  `Role::Switch`; the forwards tab host expires the start-up toasts
  before clicking Save (they covered the footer). 103 of 104 app tests
  pass; `ui_tests::sftp::file_pane_end_to_end` failed before the facelift
  (a folder-job assertion) and still does.
- **README screenshots**: the README has none to regenerate; the renders
  live in `EZSSH_UI_SHOTS` output only. Three Lucide icons were added
  (`chevron-up`, `search`, `layout-panel-left`), `layout-grid` removed.
- The terminal's own context menu (`ezssh-term`) uses `Frame::menu` and
  follows the egui visuals; it was not restyled by hand.
