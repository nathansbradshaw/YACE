# Charting assistant

The assistant is optional. Open a project, select **Tools → Chart assistant**,
then choose **Connection**.

| Connection | Setup |
| --- | --- |
| Codex CLI | Install and sign in through the CLI; YACE uses that session. |
| Claude Code CLI | Install and sign in through the CLI; YACE uses that session. |
| Ollama | Start Ollama and choose an installed model. Choose a local model for local processing. |
| OpenAI or Claude API | Enter a key and a model available to that account. Keys stay in memory for the session. |

Leave the model blank to use a CLI provider's default. API and subscription
access are separate; the selected account's limits apply.

## Ask and review

Ask about timing, patterns or difficulty, or choose **Review next flags**.
The panel uses the open project and selected track. You can keep editing while
a request runs.

Read each proposed edit before choosing **Apply this edit**. Suggestions apply
to specific supplied notes; after the chart changes, stale suggestions must be
requested again. Undo is beside the note-editing tools.

Requests contain text context, including nearby notes, flags, song information
and sometimes lyrics or local paths. Audio bytes are not uploaded. The assistant
has not listened to the recording; check advice against the audio yourself.
See [Privacy](PRIVACY.md) before using a hosted connection for private material.

Switching projects clears the conversation. Closing the panel keeps it for the
current app session. Editing and exporting work without an assistant connection.
