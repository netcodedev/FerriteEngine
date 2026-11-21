# Asset-Browser 2

Plan vom 2026-11-21, am selben Tag umgesetzt (noch nicht committet). Der
Abschnitt „Umgesetzt“ am Ende sagt, wo es vom Plan abweicht.

## Ziel

Der Asset-Browser (unten angedockt, `crates/editor/src/editor/asset_browser.rs`)
wird vom Baum zur Kachelansicht mit Kategorien, Suche, Vorschaubildern und
Kontextmenü. Anlegen, Umbenennen und Löschen gehen direkt im Browser.

## Entscheidungen (2026-11-21)

- Löschen in den System-Papierkorb (freedesktop, `trash`-Crate).
- Packs/Animationsdateien mit mehreren Teilen: Pfeil auf der Karte klappt
  die Teile als kleinere Karten direkt dahinter auf, jede einzeln ziehbar.
- Vorschaubilder auch für Modelle (erste Variante, LOD0).
- Cache im Projekt (umgesetzt in `.ferrite-cache/thumbnails`, das schon in
  `.gitignore` steht).

## Aufbau

```
┌ Assets ───────────────────────────────────────────────────────────────┐
│ Alle          │ assets › materials › bark      [🔍 Suchen…]  [+ Neu ▾] [Import] [⟳] │
│ Modelle   112 │ ┌────┐ ┌────┐ ┌────┐ ┌────┐ ┌────┐                      │
│ Texturen   64 │ │ 📁 │ │ 📁 │ │ ◉  │ │ ◉  │ │ ▦  │                      │
│ Materialien387│ │trees│ │rock│ │Bark│ │Leaf│ │bark│                      │
│ Mat.-Graphen 1│ └────┘ └────┘ └────┘ └────┘ └────┘                      │
│ Animationen 9 │                                                        │
│ Scatter Gr. 3 │ Details: Bark · Material · assets/materials/bark/…      │
└───────────────┴───────────────────────────────────────────────────────┘
```

- **Seitenleiste links** (~150 px): „Alle“ und je eine Kategorie pro
  `AssetKind` (Modelle, Texturen, Materialien, Material-Graphen,
  Animationen, Scatter Graphs) mit Anzahl. Die gewählte Kategorie bleibt pro
  Projekt gespeichert.
- **Kopfzeile:** Brotkrumen-Pfad des aktuellen Ordners (klickbar), Suchfeld,
  „Neu ▾“, „Import“, „Neu einlesen“. Die alten Knöpfe „Material“ und „Graph“
  entfallen, „Edit“ wandert ins Kontextmenü (und bleibt Doppelklick).
- **Kachelraster:** Ordner zuerst, dann Assets, beide nach Name sortiert.
  Eine Kategorie zeigt nur Ordner, die (auch tiefer) Assets dieser Art
  enthalten. Doppelklick auf einen Ordner öffnet ihn, auf ein Asset den
  passenden Editor. Ziehen in die Szene/Inspector-Felder geht wie bisher.
- **Kachel:** Vorschaubild oder Art-Symbol, Name (gekürzt, voller Name im
  Tooltip), kleine Art-Beschriftung. Modell-Packs und Animationsdateien mit
  mehreren Teilen tragen eine Zahl („3 Varianten“) — siehe offene Frage.
- **Detailzeile** unten: wie bisher eine Zeile zum ausgewählten Asset.
- **Suche:** filtert die aktuelle Kategorie über alle Unterordner (flache
  Trefferliste mit Ordner als Unterzeile), Groß/klein egal, auf Name und
  Dateiname. Leeres Feld → wieder Ordneransicht.

## Vorschaubilder

Einmal erzeugt, als PNG zwischengespeichert, nur neu bei Änderung.

| Art | Bild |
|---|---|
| Textur | Quelldatei auf der CPU auf 128 px verkleinert (`image`) |
| Material | Kugel mit dem Material, Vorschau-Licht, offscreen gerendert (`gfx::RenderTarget` + `read_pixels`, wie das Impostor-Baken) |
| Material-Graph | Kugel mit den Standardwerten des Graphen |
| Modell | erste Variante, LOD0, eingerahmt wie in der Asset-Vorschau (Frage 3) |
| Animation, Scatter Graph, Ordner | Symbol |

- Cache: `<projekt>/.ferrite/thumbnails/<id>.png` plus Schlüssel (Hash aus
  Quelldatei-Größe+Zeit, Sidecar-Inhalt und bei Materialien den Schlüsseln
  der verwendeten Graphen/Texturen). Passt der Schlüssel, wird nur geladen.
- Gerendert wird auf dem Render-Thread, ein paar pro Frame, nur für
  sichtbare Kacheln; bis dahin steht das Symbol da.
- Speichern eines Materials/Graphen/einer Textur macht den Schlüssel ungültig.

## Kontextmenü (Rechtsklick)

- **Auf ein Asset:** Öffnen · Umbenennen · Duplizieren · Pfad kopieren ·
  Im Dateimanager zeigen · Löschen…
- **Auf einen Ordner:** Öffnen · Umbenennen · Löschen…
- **Auf freie Fläche:** Neu ▸ (Ordner, Material, Material-Graph,
  Scatter Graph, …) · Importieren… · Neu einlesen

„Neu ▾“ in der Kopfzeile zeigt dasselbe Untermenü; die zur Kategorie
passende Art steht oben. Neues entsteht **im aktuellen Ordner** (nicht mehr
fest in `assets/materials`) und geht sofort in die Umbenennung.
`material::asset::create` bekommt dafür einen Zielordner.

## Umbenennen

Inline in der Kachel. Benennt Quelldatei, Sidecar und Anzeigenamen gleich
um; Verweise laufen über die Id und bleiben heil. Ordner umbenennen
verschiebt alles darin (Datenbank neu einlesen). Name schon vergeben →
Fehlermeldung, nichts passiert.

## Löschen

- `AssetDatabase::references_to(id)`: sucht die Id als `"<16 hex>"` in den
  Textdateien des Projekts (Sidecars und `.ron`-Quellen unter `assets/`,
  `scenes/`, `animations/`, Skripte). Das fängt alle Verweisarten ab —
  Modell-Slots → Material, Material → Graph/Textur, Scatter Graph → Modell,
  Clips → Modell, Szenen, Baum-Komponenten — ohne jede einzeln zu kennen.
  Treffer werden als Asset-Name (wenn die Datei zu einem Asset gehört) oder
  Pfad angezeigt.
- `AssetDatabase::delete(id)`: Quelldatei + Sidecar weg, Eintrag raus,
  `changes` hoch. Ein offener Editor für das Asset wird geschlossen.
- Bestätigungsdialog nennt Name, Art, Pfad und die Verweise („Wird
  verwendet von 3 Assets: …“, rot). Knöpfe „In den Papierkorb“ / „Abbrechen“.
- Ordner löschen: nennt die Zahl der Assets darin und alle Verweise von
  außerhalb des Ordners.
- Tests auf Datenbankebene: Sidecar und Quelle weg, Eintrag weg, Verweise
  werden gefunden (Material→Graph, Modell→Material, Scatter→Modell), eigene
  Sidecar zählt nicht als Verweis.

## MCP

- `editor_asset_delete {asset, confirm}`: ohne `confirm: true` nur die
  Vorschau (was, Verweise), mit `confirm` wird gelöscht.
- `editor_asset_references {asset}`, `editor_asset_rename {asset, name}`,
  `editor_asset_create {kind, folder, name?}`, `editor_folder_create {path}`.
- `editor_material_create` bleibt als Alias.

## Reihenfolge

1. Datenbank: `references_to`, `delete`, `rename`, Ordner-Operationen + Tests.
2. MCP-Tools darauf.
3. Browser-Gerüst: Seitenleiste, Kopfzeile, Kachelraster (neues
   `CardGrid`-Widget in engine-ui), Ordnernavigation, Suche, Ziehen.
4. Kontextmenü (Popup an Cursorposition, Untermenü „Neu“), Dialoge.
5. Vorschaubilder: Texturen, dann Materialien/Graphen, dann Modelle.
6. Doku in `docs/asset-pipeline.md`, MCP-Prüfung mit Screenshots im
   Demo-Projekt, dort `New material graph 1` über den Browser löschen.

## Umgesetzt (2026-11-21)

Code: `crates/editor/src/editor/asset_browser/` (`mod.rs` Panel und
Ereignisse, `view.rs` Kategorien/Karten/Suche ohne Zeichnen, `thumbnails.rs`),
`crates/editor/src/editor_layer/asset_commands.rs`, Dialoge in
`dialogs.rs`, MCP in `crates/editor/src/editor/asset_bridge.rs`,
Datenbank in `crates/engine/src/core/assets/database/manage.rs`, neue
Assets in `crates/engine/src/core/assets/create.rs`, Kontextmenü-Widget
`crates/engine-ui/src/ui/elements/context_menu.rs`.

Abweichungen vom Plan:

- Umbenennen und Anlegen fragen den Namen in einem kleinen Dialog ab
  (vorbelegt), nicht inline in der Karte.
- „Neu“ ist ein flaches Menü (Material, Material-Graph, Scatter Graph,
  Ordner) ohne Untermenü; die Art der Kategorie steht oben.
- Die Suche sucht in der ganzen Kategorie ab dem Asset-Ordner, nicht nur
  unter dem offenen Ordner.
- Vorschaubilder liegen in `.ferrite-cache/thumbnails/<id>.png` + `.key`.
  Materialien mit „auto“-Maps nehmen sie vom ersten Modell-Slot, der das
  Material nutzt (dessen Datei wird dafür geladen; die letzten vier bleiben
  im Speicher). Modelle gehen durch den Modell-Cache der Scatter-Modelle.
- Der Papierkorb ist der freedesktop-Papierkorb des Dateisystems: für
  `/mnt/devdata/dev` ist das `/mnt/devdata/dev/.Trash-1000`, nicht
  `~/.local/share/Trash`.
- Für die Prüfung ohne Klicks gibt es zusätzlich das MCP-Tool
  `editor_asset_browser` (Kategorie, Ordner, Suche, Auswahl, Kontextmenü,
  Aufklappen).
- Engine-Korrekturen, die das Offscreen-Rendern brauchte:
  `hdr::set_picture_target` (ein eigenes Ziel zählt als Ansicht, nicht als
  Schattenpass) und der Lighting-Cache in `pbr::Lighting::apply` merkt
  sich pro Programm auch die Pass-Art (vorher übernahm die Szene die
  Vorschau-Uniforms: Häuser wurden schwarz).

- Vorschaubilder werden höchstens die Hälfte der Zeit gezeichnet: nach
  einem Bild, das `t` brauchte, bekommt der Editor mindestens `t` für sich.

Offen:

- Assets zwischen Ordnern verschieben (Karte auf Ordner ziehen).
- Tastenkürzel (Entf, F2) im Browser.
- Das Bild eines gelöschten Assets bleibt im Cache liegen, bis man ihn
  leert.
