# KizuMail unter Windows verteilen

## Release erstellen

Aus PowerShell im Projektordner:

```powershell
.\build-package-clean.ps1
```

Das Skript erstellt einen optimierten Release-Build, bindet die lokale OAuth-Konfiguration in die Installer ein, kopiert die fertigen Dateien nach `build-artifacts` und entfernt danach die großen temporären Build-Verzeichnisse.

Für normale Windows-Nutzer ist die NSIS-Datei unter `build-artifacts\bundle\nsis` vorgesehen. Die MSI-Datei ist für verwaltete Installationen gedacht. `kizumail.exe` zusammen mit `oauth-clients.local.json` ist die portable Testvariante.

## Deinstallation

Der NSIS-Installer registriert KizuMail in **Windows-Einstellungen > Apps > Installierte Apps**. Die Deinstallation startet vor dem Entfernen der Programmdateien KizuMails sicheren Bereinigungsmodus. Dieser entfernt lokale Maildaten, Anhänge, WebView-Daten und die zu den eingerichteten Konten gehörenden Einträge aus dem Windows Credential Manager.

`build-artifacts\Uninstall-KizuMail.ps1` startet denselben registrierten Windows-Uninstaller direkt.

## Benachrichtigungen und Hintergrundbetrieb

Nach der Installation kann unter **Einstellungen** Folgendes aktiviert werden:

- Desktop-Benachrichtigungen
- Mit Windows starten
- Beim Schließen im Infobereich weiterlaufen

KizuMail verwendet IMAP IDLE, wenn der Server es unterstützt. Andernfalls wird das Konto in einem festen Intervall geprüft. Jedes Konto läuft in einem eigenen Hintergrund-Worker.

## Optionale Codesignatur

Ohne eigenes Windows-Code-Signing-Zertifikat bleibt der Build funktional, Windows kann jedoch eine SmartScreen-Warnung für einen unbekannten Herausgeber zeigen.

Mit installiertem Zertifikat:

```powershell
$env:KIZUMAIL_SIGN_CERT_THUMBPRINT = "THUMBPRINT_DES_ZERTIFIKATS"
$env:KIZUMAIL_SIGN_TIMESTAMP_URL = "http://timestamp.digicert.com"
.\build-package-clean.ps1
```

Das Build-Skript übergibt den Thumbprint nur über eine temporäre, ignorierte Konfigurationsdatei und entfernt diese nach dem Build.
