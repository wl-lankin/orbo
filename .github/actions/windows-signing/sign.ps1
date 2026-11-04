# Signs one file. Tauri calls this through bundle.windows.signCommand for every binary it signs.
param([Parameter(Mandatory)][string]$File)
$ErrorActionPreference = 'Stop'

# A fresh GitHub OIDC token per call: the build can run longer than one token lives.
$token = (Invoke-RestMethod -Uri "$env:ACTIONS_ID_TOKEN_REQUEST_URL&audience=api://AzureADTokenExchange" `
  -Headers @{ Authorization = "Bearer $env:ACTIONS_ID_TOKEN_REQUEST_TOKEN" }).value
$tokenFile = New-TemporaryFile
try {
  Set-Content $tokenFile $token -NoNewline
  $env:AZURE_FEDERATED_TOKEN_FILE = $tokenFile.FullName

  & $env:ARTIFACT_SIGNING_SIGNTOOL sign /v /fd SHA256 /tr http://timestamp.acs.microsoft.com /td SHA256 `
    /dlib $env:ARTIFACT_SIGNING_DLIB /dmdf $env:ARTIFACT_SIGNING_METADATA `
    /d $env:ARTIFACT_SIGNING_DESCRIPTION $File
  if ($LASTEXITCODE -ne 0) { throw "signtool failed with exit code $LASTEXITCODE for $File" }
}
finally {
  Remove-Item $tokenFile -ErrorAction SilentlyContinue
}
