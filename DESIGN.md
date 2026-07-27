# KizuMail Design System

## 1. Zweck

Dieses Dokument ist die verbindliche visuelle Spezifikation für KizuMail.

KizuMail ist ein moderner, lokaler, datenschutzfreundlicher E-Mail-Client. Die Anwendung soll nicht wie Outlook, Thunderbird, Windows Mail oder klassische Business-Software wirken. Sie soll wie hochwertige moderne Produktivitätssoftware wirken: ruhig, dunkel, klar, schnell, hochwertig und visuell eigenständig.

Die Referenzbilder sind nicht als Beispiel für echte E-Mail-Inhalte zu verstehen. Sie zeigen die gewünschte Designsprache, Layoutlogik, Hierarchie, Kartenstruktur, Abstände, Rundungen, Farben und Präsentation innerhalb der App.

Wichtig: Die App soll echte E-Mails nicht roh wie eine Webseite anzeigen. Echte HTML-Mails sind nur die Datenquelle. KizuMail kontrolliert die Präsentation.

---

## 2. Design-Ziel

Die App soll wirken wie eine Mischung aus:

- Arc Browser
- Linear
- Raycast
- moderne macOS-Apps
- Apple Mail, aber deutlich moderner
- Notion, aber dunkler und technischer

Die App soll nicht wirken wie:

- Outlook
- Thunderbird
- Windows Mail
- Gmail Web
- Admin-Dashboard
- klassische Business-Software
- rohe Browser-HTML-Mail-Darstellung

---

## 3. Grundprinzipien

1. Cards statt flacher Listen.
2. Viel Weißraum.
3. Große Rundungen.
4. Weiche Schatten.
5. Ruhige dunkle Flächen.
6. Lila als Akzentfarbe.
7. Wenige Farben.
8. Keine Tabellenoptik.
9. Keine engen Layouts.
10. Keine kleinen gedrängten Controls.
11. Keine rohe HTML-Mail als Hauptdarstellung.
12. Jede Mail soll so wirken, als wäre sie von KizuMail neu komponiert.

---

## 4. Visuelle Prioritäten

Bei Konflikten gilt diese Reihenfolge:

1. Layout
2. Größenverhältnisse
3. Weißraum
4. Typografie
5. Kartenstruktur
6. Farben
7. Schatten
8. Rundungen
9. Animationen
10. Icons
11. Funktionalität

Funktionalität darf nicht zerstört werden, aber visuelle Struktur ist bei UI-Arbeiten die höchste Priorität.

---

## 5. Layout

Desktop-Hauptlayout:

- Fenster: vollständige Desktop-App, dunkles Theme
- Vier visuelle Bereiche:
  1. Linke Sidebar
  2. Nachrichtenliste
  3. Hauptbereich / geöffnete E-Mail
  4. Rechte Informationsleiste

Empfohlene Breiten:

- Sidebar: 250–270 px
- Nachrichtenliste: 390–430 px
- Hauptbereich: flexibel, mindestens 620 px
- Rechte Informationsleiste: 240–280 px

Die Bereiche sollen klar getrennt sein, aber nicht hart wirken. Trenner sind subtil und dunkel.

---

## 6. Farben

Grundfarben:

```css
--bg-app: #070A0F;
--bg-sidebar: #0B0E17;
--bg-panel: #0F1320;
--bg-panel-soft: #131827;
--bg-card: #171B2A;
--bg-card-hover: #1D2235;
--bg-card-active: #241B4A;

--border-soft: rgba(255, 255, 255, 0.06);
--border-medium: rgba(255, 255, 255, 0.10);

--text-primary: #F5F7FB;
--text-secondary: #AAB0C0;
--text-muted: #6F7788;

--accent: #7C3AED;
--accent-soft: #8B5CF6;
--accent-glow: rgba(124, 58, 237, 0.35);

--success: #2EE98C;
--warning: #F6C945;
--danger: #FF5C7A;
```

Regel:

- Keine grellen Farben außer für Icons/Brand-Erkennung.
- Lila ist der Hauptakzent.
- Grün nur für Status "sicher/gefunden/aktiv".
- Rot nur für gefährliche Aktionen.

---

## 7. Typografie

Empfohlen:

- System Font Stack:
  ```css
  font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  ```

Typografie-Hierarchie:

- App-Name: 20–22 px, 700
- Haupttitel geöffnete Mail: 24–32 px, 700–800
- Panel-Titel: 18–20 px, 700
- Card-Titel: 15–16 px, 650–700
- Body-Text: 14–16 px, 400–500
- Meta-Text: 12–13 px, 500
- Sidebar-Items: 14–15 px, 600

Zeilenhöhe:

- Überschriften: 1.1–1.2
- Body: 1.45–1.65
- Listen: 1.25–1.4

---

## 8. Rundungen

Mindestwerte:

- App-Fenster / große Panels: 18–22 px
- Karten: 16–18 px
- Buttons: 12–999 px je nach Form
- Inputs: 14–16 px
- Badges: 999 px
- kleine Icon-Buttons: 10–12 px

Keine harten 4px/6px-Rundungen für Hauptkomponenten.

---

## 9. Schatten und Tiefe

Cards sollen subtil schweben.

Empfohlene Schatten:

```css
--shadow-card: 0 16px 40px rgba(0, 0, 0, 0.28);
--shadow-soft: 0 8px 24px rgba(0, 0, 0, 0.20);
--shadow-glow: 0 0 32px rgba(124, 58, 237, 0.16);
```

Regeln:

- Schatten weich, nicht hart.
- Aktive Cards dürfen einen leichten lila Glow haben.
- Keine übertriebenen Neon-Effekte.

---

## 10. Sidebar

Ziel: hochwertig, ruhig, nicht gedrängt.

Eigenschaften:

- Breite: 250–270 px
- Padding links/rechts: 18–22 px
- Section-Abstand: 24–34 px
- Sidebar-Item-Höhe: 38–44 px
- Icon-Größe: 18–22 px
- Aktives Item:
  - lila getönter Hintergrund
  - runde Card
  - optional Count-Badge rechts

Button "Neue E-Mail":

- volle Breite
- Höhe 46–52 px
- Gradient oder solides Lila
- Radius 12–16 px
- Schrift 14–15 px, 700
- Icon/Button rechts abgetrennt oder integriert

---

## 11. Suchleiste

Suchleiste soll wie eine Premium-Komponente wirken.

Eigenschaften:

- Höhe: 44–52 px
- Radius: 14–18 px
- Hintergrund: dunkle Card-Fläche
- Rand: subtil
- Innenabstand links/rechts: 16–18 px
- Icon links
- zusätzliche Filter-Buttons rechts möglich
- Hover/Focus mit lila Border/Glow

---

## 12. Nachrichtenliste

Die Nachrichtenliste ist eine Card-Liste, keine Tabelle.

Jede Mail:

- Radius: 16–18 px
- Padding: 16–20 px
- Abstand zwischen Mails: 10–14 px
- Mindesthöhe: 74–92 px
- Logo/Avatar: 44–52 px
- Titel: 15–16 px, 700
- Betreff: 14–15 px, 600
- Vorschau: 13–14 px, gedämpft
- Datum/Uhrzeit rechts oben
- Ungelesenpunkt rechts oder als Badge
- Hover: Hintergrund heller + leichte Translation/Glow
- Active: lila getönte Fläche + lila Border links oder Glow

Nicht erlaubt:

- eng gepackte Listenzeilen
- Tabellenlayout
- minimale 1px-Trenner als Hauptstruktur
- zu kleine Logos

---

## 13. Geöffnete E-Mail

Wichtigster Punkt:

KizuMail soll nicht einfach rohe HTML-Mails anzeigen.

Standardansicht: "Readable / KizuMail View"

Die App extrahiert relevante Daten:

- Absender
- Domain
- Betreff
- Zeit
- Textinhalt
- wichtige Links
- Newsletter-Header
- Attachments
- Sicherheitsinformationen

Dann rendert KizuMail daraus eine eigene hochwertige Ansicht.

Rohes Original-HTML ist nur über einen Umschalter "Original" sichtbar.

### Aufbau

1. Top Toolbar
2. Mail Header
3. Badges
4. Newsletter-/Sicherheitsbanner
5. KizuMail Content Card
6. Links/Attachments
7. Bottom Actions

### Header

- Titel groß: 24–32 px
- Badges daneben oder darunter
- Absender mit großem Logo/Avatar
- Zeit rechts
- Aktionen rechts

### Content Card

- Max Breite: 700–820 px
- Zentriert oder leicht linksbündig im Contentbereich
- Radius: 18–22 px
- Padding: 28–40 px
- Hintergrund: dunkler Verlauf oder Card-Fläche
- Rand: subtil lila/dunkel
- Schatten: weich

Die Content Card darf Markenfarben aufnehmen, aber kontrolliert und gedämpft.

Beispiel:
Spotify-Mail → grüner Akzent erlaubt, aber weiterhin KizuMail-Layout.
Apple-Mail → monochrom/neutral.
GitHub-Mail → dunkle Security Card.
Amazon-Mail → Order/Deal Card.

---

## 14. Readable View vs Original View

Die App braucht zwei Modi:

### Lesbar

- Standardmodus
- KizuMail rendert die E-Mail neu
- Tracking-Pixel blockiert
- externe Bilder standardmäßig blockiert oder kontrolliert
- Layout sauber und einheitlich

### Original

- zeigt die originale HTML-Mail
- deutlich als Original markiert
- Tracking weiterhin blockiert, falls möglich
- weniger wichtig fürs Design

Der Nutzer soll immer merken: Standard ist KizuMail, nicht Browser-HTML.

---

## 15. Newsletter-Erkennung

Newsletter-Banner:

- große horizontale Card
- Radius 14–18 px
- Padding 16–20 px
- Text links
- Button rechts
- Button "Abmelde-Vorschau" oder "Abbestellen"
- Status-Badges: Newsletter, HTML, Sicher, Gefunden

Kein kleiner technischer Hinweis. Es soll wie ein Kernfeature aussehen.

---

## 16. Rechte Informationsleiste

Die rechte Leiste soll nicht wie ein Inspector wirken.

Sie soll aus Cards bestehen:

1. Absender-Card
2. Informationen-Card
3. Newsletter-Status-Card
4. E-Mail-Aktivitätsdiagramm
5. Optionen-Card

Eigenschaften:

- Padding: 18–22 px
- Card Radius: 16–18 px
- Abstand: 16–22 px
- große Zahlen
- klare Statusfarben
- viel Luft

---

## 17. Buttons

Button-Regeln:

- Radius: 12–999 px
- Padding: 10–18 px
- Hauptbutton: lila
- Nebenbutton: dunkle Card-Fläche
- Ghost-Button: transparent mit Hover
- Icon-Button: quadratisch/rund, 36–42 px

Hover:

- Hintergrund heller
- leichte Translation `translateY(-1px)`
- subtiler Schatten

Active:

- minimal dunkler
- keine harten Effekte

---

## 18. Badges

Badges:

- Radius: 999 px
- Padding: 4–8 px
- Schrift: 11–12 px, 650
- Hintergrund: lila transparent
- Text: hell-lila
- Beispiele:
  - Newsletter
  - HTML
  - Sicher
  - Gefunden
  - Hoch
  - Original
  - Lesbar

---

## 19. Icons und Logos

- Icons einheitlich.
- Keine zufällig gemischten Icon-Stile.
- Lucide/Phosphor/Heroicons einheitlich verwenden.
- Brand-Logos dürfen farbig sein.
- Platzhalter-Avatare hochwertig gestalten, nicht generisch grau.

---

## 20. Animationen

Dezent, aber vorhanden.

Empfohlen:

```css
transition: background 160ms ease, border-color 160ms ease, transform 160ms ease, box-shadow 160ms ease, opacity 160ms ease;
```

Hover:

- Cards: `translateY(-1px)`
- Buttons: leichte Helligkeit
- aktive Elemente: Glow

Keine übertriebenen Bounces.
Keine langen Animationen.
Keine verspielten Effekte.

---

## 21. Abstände

Grundsystem:

- 4 px Basis
- bevorzugte Werte: 8, 12, 16, 20, 24, 32, 40

Keine zufälligen Werte.

Hauptbereiche:

- Panel Padding: 18–24 px
- Card Padding: 16–24 px
- Mail Content Padding: 28–40 px
- Section Gap: 24–36 px

---

## 22. Responsive Verhalten

Desktop zuerst.

Mindestbreite:

- unter 1100 px: rechte Infoleiste ausblendbar
- unter 850 px: Mailliste und Mailansicht als Umschaltansicht
- mobile später, nicht MVP

---

## 23. Verbotene Muster

Nicht verwenden:

- rohe Tabellenansichten
- Standard-Browser-HTML als Hauptdesign
- helle harte Borders
- kleine gedrängte Buttons
- unruhige Icon-Mischung
- zufällige Fonts
- zu viele Farben
- unnötige Linien
- Windows-95/Classic/Business-Look
- Outlook/Thunderbird-Optik

---

## 24. Entwicklungsregel für Codex

Vor jeder größeren UI-Änderung:

1. Referenzbild ansehen.
2. Aktuelle App per Screenshot prüfen.
3. Unterschiede konkret benennen.
4. Erst dann implementieren.
5. Nach Änderung neuen Screenshot erstellen.
6. Screenshot mit Referenz vergleichen.
7. Weiter iterieren, bis Layout und Proportionen deutlich näher sind.

Keine Mini-Änderungen als "fertig" markieren, wenn das Grundlayout noch abweicht.

---

## 25. Definition of Done für UI-Arbeiten

Eine UI-Aufgabe ist erst fertig, wenn:

- Layout sichtbar näher an der Referenz ist.
- Abstände großzügiger sind.
- Cards klar erkennbar sind.
- rohe HTML-Mail nicht mehr die Standardoptik dominiert.
- Sidebar, Mailliste, Content und Rechte Leiste dieselbe Designsprache haben.
- Der Screenshot der App beim direkten Vergleich nicht mehr wie Thunderbird/Outlook wirkt.
- Mindestens ein Screenshot nach der Änderung erzeugt wurde.
- Auffällige Unterschiede zur Referenz dokumentiert wurden.

---

## 26. Nächster großer Umbau

Der nächste wichtige Schritt ist kein Feinschliff.

Der nächste Schritt ist ein Frontend-Redesign.

Codex soll das bestehende UI nicht "verbessern", sondern die visuelle Struktur neu aufbauen:

- Sidebar neu als Premium Sidebar
- Mailliste neu als Card List
- E-Mail-Ansicht neu als KizuMail Readable View
- Rechte Leiste neu als Card-based Sender Intelligence Panel
- Original HTML nur als optionaler Modus
