# Privacy and data

## Local files

YACE decodes recordings and runs transcription models on your computer. Projects
retain source copies, edits and review history. Generation jobs retain inputs,
model results and worker logs so you can resume after a failure.

Treat project and job folders as private. They can contain recordings, lyrics,
song information and local file paths. Review exports before sharing them.

Desktop panics are logged locally under `~/Library/Logs/YACE`. These logs can
contain paths and error details. Check them before attaching them to an issue.

## Optional network requests

| Action | What leaves your computer |
| --- | --- |
| Download a model | A request to the model host, including the requested asset |
| Look up lyrics | Song title, artist, album when supplied, and duration sent to LRCLIB |
| Ask the assistant | Text context sent to the selected provider or CLI |
| Publish collaboration data | Proposals, discussions and review history sent to the chosen Git remote or shared folder |

Assistant text can include lyrics, song information, local paths, note IDs and
review evidence. YACE does not include audio bytes in assistant requests.
Provider and CLI retention policies still apply. An Ollama connection can use
either local or cloud models; choose a local model when you want local processing.

API keys stay in application memory for the current session. YACE does not save
them in projects or settings. CLI providers use their own stored authentication.
See [AI_ASSISTANT.md](AI_ASSISTANT.md) for context limits and edit controls.

Collaboration publishes only when you invoke its publishing operations. Its
review format does not include song audio. Check discussions and proposal text
for anything you do not want to share.
