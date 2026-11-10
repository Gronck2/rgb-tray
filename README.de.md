# RGB Tray

[English](README.md) · [Русский](README.ru.md) · [简体中文](README.zh-CN.md) · [Español](README.es.md) · **Deutsch** · [Français](README.fr.md) · [Português (Brasil)](README.pt-BR.md) · [日本語](README.ja.md)

**Website:** [rgb-tray.gronck.com](https://rgb-tray.gronck.com/de/)

Eine kleine Windows-Tray-App, die die RGB-Beleuchtung von Tastaturen, Mäusen und Lampen direkt über HID steuert (und Tuya-Lampen über das lokale Netzwerk) – ohne dass Herstellersoftware (G HUB, Synapse, Armoury Crate, SignalRGB) im Hintergrund läuft.

**Dein Gerät fehlt in der Liste? Füge es selbst hinzu.** Öffne dieses Repository in [Claude Code](https://claude.com/claude-code) und gib `/add-device` ein: Claude findet das Protokoll, schreibt ein Profil oder einen Treiber und testet es mit dir, ohne dass du Rust oder HID kennen musst ([so funktioniert es](#das-eigene-gerät-mit-claude-hinzufügen)).

<p>
  <img src="docs/screenshots/panel-dark.png" width="300" alt="Tray-Panel">
  <img src="docs/screenshots/editor-dark.png" width="520" alt="Editor für einzelne Tasten">
</p>

## Unterstützte Geräte

| Gerät | Verbindung | Was funktioniert |
|---|---|---|
| Logitech G915 TKL | LIGHTSPEED-Empfänger | Statisch, Atmen, Spektrum, aus; Farben pro Taste; Akku |
| Razer Naga V2 Pro | HyperSpeed-Dongle / Dock | Getrennte Zonen für Logo und Mausrad; Effekte; Akku |
| Razer Mouse Dock Pro | USB | Statisch, Atmen, Spektrum, aus |
| Rund 90 weitere Razer-Mäuse, -Tastaturen und -Zubehörteile (Basilisk, DeathAdder, Viper, Cobra, BlackWidow, Huntsman, Ornata, Firefly, Goliathus, Base Station…) auf Basis der Daten von OpenRazer – [Liste](profiles) | USB / Dongle | Statisch, Atmen, Spektrum, aus; Zonen bei Mäusen; Akku. **Ungetestet**: Die Karte weist darauf hin, und ein Bericht ist willkommen. Kabellose HyperSpeed-Tastaturen vorerst nur per Kabel. |
| ASUS-Laptoptastatur (Aura) | Eingebaut | Statisch, Atmen, Spektrum, aus; Tastaturzonen und Gehäuseleuchten über LampArray |
| Geekboard MacroPad v2 (QMK/VIA) | USB | Statisch, Atmen, Spektrum, aus |
| Andere QMK-Tastaturen mit VIA und RGB (Keychron, Eigenbauten…) | USB | Statisch, Atmen, Spektrum, aus, Helligkeit (Atmen und Spektrum werden von der App gezeichnet) |
| Geräte mit dem Windows-Standard „Dynamische Beleuchtung“ (HID LampArray) | USB / eingebaut | Statisch, Atmen, Spektrum, aus, Zonen vom Gerät (Atmen und Spektrum werden von der App gezeichnet) |
| Tuya-WLAN-Lampen (Smart Life), z. B. LINUS A60 | WLAN, lokales Netzwerk | Farbe, Weiß (eine weißliche Farbe wählt die weißen LEDs und deren Farbton), Atmen, Spektrum, aus; wird nicht mit dem Bildschirm dunkel. Benötigt den lokalen Schlüssel der Lampe, siehe unten |

Andere Modelle, die dieselben Protokolle sprechen, funktionieren möglicherweise, sind aber ungetestet.

### Tuya-Lampen

RGB Tray steuert eine Tuya-Lampe über das lokale Netzwerk (Protokoll 3.3) mit dem *lokalen Schlüssel* (local key) der Lampe, den nur die Tuya-Cloud herausgibt. Besorgen Sie ihn einmalig, zum Beispiel mit [tinytuya](https://github.com/jasonacox/tinytuya) (`python -m tinytuya wizard`) oder über den Smart-Life-QR-Login von `tuya-device-sharing-sdk`. Tragen Sie die Lampen dann in `%APPDATA%\rgbtray\tuya.toml` ein:

```toml
[[device]]
name = "Schreibtischlampe"
id = "bf1234567890abcdef12"
local_key = "0123456789abcdef"
```

Die App findet die Lampe im lokalen Netzwerk selbst und merkt sich ihre Adresse (`local_ip`). Die Lampe muss in **Smart Life** oder **Tuya Smart** eingebunden sein: Konten von Markenablegern dieser App können keine Schlüssel herausgeben. Eine Lampe akzeptiert jeweils nur eine lokale Verbindung: Solange die Smart-Life-App auf einem Smartphone geöffnet ist, kann RGB Tray sie nicht erreichen.

### Ein neues Gerät anfragen

Rechtsklick auf das Tray-Symbol → **Request device support…** („Geräteunterstützung anfragen…“). RGB Tray listet Ihre HID-Geräte auf, markiert diejenigen, deren Beleuchtung sich möglicherweise steuern lässt, kopiert den Bericht in die Zwischenablage und öffnet das [Anfrageformular](../../issues/new?template=device-request.yml). Geben Sie das Modell ein und fügen Sie den Bericht ein. Seriennummern im Bericht werden gekürzt.

Ein Razer- oder QMK/VIA-Gerät oder eines mit einem einfachen Protokoll lässt sich oft sofort hinzufügen, ohne neue Version: Beschreiben Sie es in einer kleinen Profildatei ([docs/profiles.md](docs/profiles.md)).

### Das eigene Gerät mit Claude hinzufügen

Mit [Claude Code](https://claude.com/claude-code) können Sie ein Gerät selbst zum Laufen bringen, ohne Rust oder HID zu kennen. Klonen Sie dieses Repository, öffnen Sie Claude Code darin und geben Sie `/add-device` ein. Claude liest den Gerätebericht, sucht nach dem Protokoll (OpenRGB, OpenRazer, ein USB-Mitschnitt der Hersteller-App), schreibt ein Profil oder einen Treiber und testet es gemeinsam mit Ihnen: Da es die Beleuchtung nicht sehen kann, fragt es nach jedem Schritt, was das Gerät getan hat. Befehle, die einem Gerät schaden könnten (Firmware-Updates, Speichern im Flash-Speicher), sind tabu. Wenn alles funktioniert, kann Claude das Ergebnis als Pull Request einreichen, damit es für alle fest eingebaut wird.

## Funktionen

Die Oberfläche der App gibt es auf Englisch und Russisch.

- Ein Panel für alle Geräte: Farbe, Effekt, Helligkeit – für alles auf einmal oder pro Gerät.
- Globale Helligkeit plus eine zusätzliche Helligkeit pro Gerät.
- Geschwindigkeit von Atmen und Spektrum, global und pro Gerät.
- Firmware-Effekte einer Tastatur in ihrem Tasteneditor, neben den Voreinstellungen: Welle und Wellenring (Ripple) auf der Logitech G915. Sobald eine Taste eingefärbt wird, kehren die Tastenfarben zurück.
- Editor für einzelne Tasten der Logitech-Tastatur: Pinsel, Pipette, Regenbogen, Zweifarbverlauf, Voreinstellungen für WASD und nur Buchstaben, Hervorheben eines Wortes, gespeicherte Schemata, ANSI/ISO-Layouts.
- Voreinstellungen mit globalen Tastenkürzeln (einschließlich F13–F24 mit Ctrl/Alt/Shift/Win).
- Ein globales Tastenkürzel pro Gerät, das es aus- und wieder in den vorherigen Zustand einschaltet (die Tastatur-Schaltfläche hinter dem Namen auf seiner Karte: F13–F24 oder ein beliebiges Kürzel), z. B. für eine Zimmerlampe. Voreinstellungen sind davon nicht betroffen.
- Mehrere identische Geräte mit getrennten Einstellungen; Geräte werden anhand ihrer Seriennummer erkannt.
- Getrennte Geräte bleiben mit ihren Einstellungen in einem Archiv und kehren zurück, sobald sie wieder angeschlossen werden.
- Status „No link“ („Keine Verbindung“), wenn ein kabelloses Gerät schläft oder ausgeschaltet ist; die Farben werden beim Aufwachen erneut angewendet.
- Wendet die Beleuchtung nach dem Ruhezustand (einschließlich Modern Standby) und beim Anschließen von Geräten erneut an.
- Updates: Einmal am Tag fragt die App GitHub nach dem neuesten Release; eine neuere Version erscheint als Karte im Panel und lässt sich mit einem Klick installieren. Die Prüfung kann im Tray-Menü abgeschaltet werden.
- Folgt dem Bildschirm: Die Beleuchtung wird gedimmt, wenn Windows den Bildschirm dimmt, und geht aus, solange der Bildschirm aus ist (Leerlauf, gesperrt, Ruhezustand). Lässt sich im Tray-Menü abschalten.
- Warnungen bei niedrigem Akkustand bei 10 % und 5 %.
- Das Tray-Symbol zeigt die aktuelle Farbe.
- Angehefteter Modus: Das Panel bleibt geöffnet im Vordergrund und lässt sich an seiner Kopfzeile verschieben.
- Export und Import aller Einstellungen in eine Datei. Voreinstellungen lassen sich einzeln weitergeben: Rechtsklick auf eine Voreinstellung → Save to a file… („In Datei speichern…“), und der Empfänger lädt sie über Load settings or a preset… („Einstellungen oder Voreinstellung laden…“) im Tray-Menü; sie wird auf seine Geräte derselben Modelle angewendet (auf alle, falls er mehrere hat), und die App meldet, welche Modelle aus der Datei er nicht besitzt.
- Englische und russische Oberfläche: Russisch, wenn Windows oder eines seiner Tastaturlayouts Russisch ist, sonst Englisch; im Tray-Menü umschaltbar.
- Helles und dunkles Design, passend zu Windows.

## Installation

1. Laden Sie `RGBTray-Setup-<version>.exe` unter [Releases](../../releases) herunter.
2. Führen Sie die Datei aus. Die App installiert sich für den aktuellen Benutzer ohne Administratorrechte nach `%LOCALAPPDATA%\Programs\RGB Tray` und startet.
3. Entfernen lässt sie sich über Windows-Einstellungen → Apps.

Das Installationsprogramm ist noch nicht signiert, daher zeigt Windows SmartScreen möglicherweise eine Warnung an (Weitere Informationen → Trotzdem ausführen). Die Einstellungen werden in `%APPDATA%\rgbtray` gespeichert und bleiben bei Updates und Neuinstallationen erhalten. Der Ordner `backups` dort enthält Kopien der Einstellungen der letzten 14 Tage und von vor jedem Update; wird die Einstellungsdatei beschädigt, nimmt die App selbstständig die letzte funktionierende Kopie.

Schließen Sie die Beleuchtungssoftware der Hersteller, bevor Sie RGB Tray verwenden: Zwei Programme, die auf dasselbe Gerät schreiben, kommen sich gegenseitig in die Quere. Unter Windows 11 sorgt der Dienst „Logitech LampArray“ (Dynamische Beleuchtung) dafür, dass die G915 Effekte ignoriert; die App weist darauf hin und bietet an, ihn zu deaktivieren.

## Befehlszeile

| Befehl | Was er tut |
|---|---|
| `rgbtray --diagnose [file.txt]` | Schreibt den Gerätebericht: App- und Windows-Version, unterstützte Geräte und alle HID-Geräte, gruppiert mit ihren Schnittstellen, wahrscheinliche Beleuchtungs-Controller zuerst. Ohne Dateinamen wird er unter `%APPDATA%\rgbtray\rgbtray-diagnose.txt` gespeichert und im Editor geöffnet. Auch im Tray-Menü verfügbar. |
| `rgbtray --hid-send <out.txt> <vid:pid[:page[:usage]]> <feature|output|get> "<hex>"…` | Sendet rohe HID-Reports und hängt die Antworten an die Datei an, um das Protokoll eines Geräts zu ermitteln ([docs/profiles.md](docs/profiles.md#hid-any-device-with-a-simple-protocol)). |
| `rgbtray --profile-check <out.txt> <profile.toml>` | Liest eine Profildatei, ohne ein Gerät anzusprechen, und schreibt, was die App aus jedem Modell macht oder was daran falsch ist. |
| `rgbtray --profile-test <out.txt> <profile.toml>` | Spielt Farben und Effekte auf den Geräten eines noch nicht installierten `[hid]`-Profils ab und protokolliert jeden Schritt. |
| `rgbtray --show` | Startet mit geöffnetem und angeheftetem Panel. |
| `rgbtray --uninstall` | Entfernt die App (die Einstellungen bleiben erhalten). |

## Aus dem Quellcode bauen

Voraussetzungen: Windows 10/11 und stabiles [Rust](https://rustup.rs) mit der Toolchain `x86_64-pc-windows-gnu` oder `-msvc`. Öffnen Sie nach der Installation von rustup ein neues Terminal, damit `cargo` im PATH ist.

```powershell
cargo build --release            # target\release\rgbtray.exe
.\scripts\release.ps1            # installer in dist\RGBTray-Setup-<version>.exe
.\scripts\release.ps1 -Install   # build and install/update silently
```

Das Installationsprogramm ist dieselbe ausführbare Datei: Heißt sie `RGBTray-Setup-*.exe` (oder wird sie mit `--install` gestartet), installiert sie sich selbst.

## Mitwirken

Geräteberichte, Tests auf Ihrer Hardware, Fehlerberichte und Pull Requests sind willkommen. [CONTRIBUTING.md](CONTRIBUTING.md) erklärt, wie man die App baut und prüft, wie man ein Gerät hinzufügt und was ein Pull Request braucht.

## Danksagung

Die Geräteprotokolle wurden aus diesen Projekten gelernt; RGB Tray enthält keinen Code daraus:

- [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB) – Beleuchtung von Logitech, Razer und ASUS Aura
- [OpenRazer](https://github.com/openrazer/openrazer) – Razer-Protokoll
- [Solaar](https://github.com/pwr-Solaar/Solaar) – Logitech HID++
- [QMK](https://qmk.fm) und [VIA](https://www.caniusevia.com) – Beleuchtungsprotokoll des MacroPad

## Lizenz

Lizenziert nach Ihrer Wahl unter [MIT](LICENSE-MIT) oder [Apache-2.0](LICENSE-APACHE).

Sofern Sie nicht ausdrücklich etwas anderes angeben, wird jeder Beitrag, den Sie absichtlich zur Aufnahme in dieses Projekt einreichen, im Sinne der Apache-2.0-Lizenz wie oben doppelt lizenziert, ohne zusätzliche Bedingungen oder Konditionen.

Nicht verbunden mit Logitech, Razer, ASUS, Geekboard oder Tuya. Marken gehören ihren jeweiligen Inhabern.

Dies ist eine Übersetzung; maßgeblich ist die englische README.
