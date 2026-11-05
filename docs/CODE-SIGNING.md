# Code-Signierung (Windows)

App, Hilfsprogramme (ffmpeg, ffprobe, `cli/orbo.exe`) und Installer werden mit [Azure Artifact Signing](https://learn.microsoft.com/azure/artifact-signing/) signiert. Als Herausgeber steht „Wolfgang Linz“ im Zertifikat, Microsoft hat die Identität geprüft.

## Azure

- Konto `wolfganglinz-signing` (Ressourcengruppe `code-signing`, North Europe, Endpunkt `https://neu.codesigning.azure.net`), Zertifikatprofil `wolfganglinz-public` (Public Trust). Die Zertifikate leben nur wenige Tage und werden von Azure erneuert, der Zeitstempel hält die Signatur danach gültig.
- App-Registrierung `github-code-signing` mit der Rolle „Artifact Signing Certificate Profile Signer“ nur auf diesem Konto. Sie hat kein Passwort: eine Verbundanmeldeinformation vertraut GitHub-OIDC-Tokens für `repo:wl-lankin@27097016/orbo@1425256825:environment:release`.

## GitHub

- Umgebung `release`, darin laufen der Windows-Job des Release-Workflows und der Prüf-Workflow.
- Repository-Variablen `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID` (IDs, keine Geheimnisse).
- `.github/actions/windows-signing` lädt den Signier-Client von Microsoft (Version und SHA-256 fest), schreibt dessen `metadata.json` und eine Tauri-Konfiguration mit `bundle.windows.signCommand`. `sign.ps1` holt für jede Datei ein frisches OIDC-Token und ruft `signtool` mit der Client-DLL auf.
- Der Release-Workflow signiert `cli/orbo.exe` gleich nach dem Bauen (Tauri signiert Ressourcen nicht) und gibt die Signier-Konfiguration an `tauri build`. Tauri signiert App, Sidecars und Installer während des Builds, die Updater-`.sig`-Dateien entstehen also über dem signierten Installer.
- `windows-signing-check.yml` baut und signiert ohne Release und prüft die Signaturen im Installer. Er läuft bei Pull Requests, die die Signierung ändern, oder von Hand.
