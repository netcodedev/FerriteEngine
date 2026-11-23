# Editor languages and settings

Plan and as-built notes (2026-11-23) for the editor's English/German texts,
the File > Settings dialog and the graphics settings' sub-categories.

## Requirements (the user's, 2026-11-23)

1. Every text in the editor interface comes from one translation file;
   English and German are supported.
2. File menu gets a "Settings" entry that opens a settings popup.
3. The language is switched there.
4. Other useful settings may go there too, but the graphics settings stay
   in the Performance panel.
5. The graphics settings get collapsible sub-categories.
6. The duplicate VSync switch goes (the frame section had one, the
   graphics settings another).

## Design

- `crates/editor/i18n/texts.ron`: one map `key -> (en, de)`, built into the
  editor (`include_str!`). Keys are `<area>.<what>` (`toolbar.file`,
  `dialogs.add_script.title`); `common.*` holds words many places share.
  `{name}` placeholders are filled by the code.
- `crate::i18n`: `tr!("key")` gives a `&'static str`,
  `tr!("key", n = value)` a `String`. A missing text falls back to
  English, a missing key shows the key (and a test fails).
- Not translated: names that are data (entities, assets, scripts,
  components' fields from reflection, project-defined graphics settings),
  log lines, MCP replies (they are for agents and stay English),
  identifiers that are saved or compared (sections are saved by id, not
  by their shown title).
- The engine's graphics settings carry German labels (the demo's options
  menu shows them); the editor shows `graphics.setting.<key>` and
  `graphics.group.<group>` from the file when there is one, else the
  engine's label.
- Language change: `i18n::generation()` moves; the editor layer rebuilds
  its panels (what they show lives in the shared state, so nothing is lost
  but scroll positions and canvas pan/zoom). The launcher builds its
  texts at start.
- Settings: `crate::preferences`, the user's own (not the project's), in
  `~/.config/ferrite/editor.ron`: language (default: the system's),
  viewport camera mouse sensitivity, invert Y, fly speed at start, Shift
  factor, field of view.
- Graphics: inside "Graphics", the preset rows stay on top; each group of
  settings (Display, World, Lighting, the project's own groups) is a
  collapsible section, open/closed state kept in the project's editor
  settings by group id. The frame section's VSync row is removed; VSync is
  the graphics setting `display.vsync` (MCP `editor_debug_views` still
  sets it).

## Checks

- Tests: the file parses, every text has both languages, every `tr!` key
  in the sources exists, placeholders match between languages.
- In the editor: switch language in File > Settings, look at every
  workspace and dialog; graphics groups fold and stay folded after a
  restart; only one VSync switch.

## As built (2026-11-23)

- About 650 texts in `crates/editor/i18n/texts.ron`, grouped by area.
  German uses the user's terms: "Scatter-Graph" (not "Streugraph").
- `asset_browser::kind_label` and `dialogs::new_asset_kind` name asset
  kinds in the user's language (the engine's `label()` stays English);
  `dialogs::kind_in_sentence` lowercases them only in English.
- The Performance panel's sections keep their old English titles as ids
  (`closed_stats_sections` in existing projects still works); graphics
  groups are folded by the engine's group name (`closed_graphics_groups`).
- Settings dialog: `editor/settings_dialog.rs`; values are applied with
  "Apply" (numbers are typed, kept in their ranges).

### Still English (texts that come from the engine, not the editor)

- Component names in the inspector and the "Add component" menu
  (`ComponentData::catalog()`), and reflected field names.
- Scatter graph nodes: titles, pins, properties, descriptions, problems
  (engine scatter graph definitions).
- Material graph nodes, parameter types, material inspector sections and
  rows (`engine/src/core/material/{graph,edit}.rs`).
- Animation condition summaries (`describe()`), `ClipDef::source_label`.
- Engine error messages passed through to the status line.
- The widget showcase (a developer demo).

The user decided on 2026-11-23 that texts coming from the engine stay
untranslated.
