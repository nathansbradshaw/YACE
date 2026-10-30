# Collaboration format

Review exchange uses portable JSON files. Audio stays outside the exchange.
Use the desktop **Collaborate** panel or `chart-engine collaborate --help` to
create, import and sync them.

## Bundles and event files

A bundle has the shape `{ "version": 1, "events": [...] }`. A shared review
directory stores each event in a separate `<event-id>.json` file. Keep complete
bundles outside that directory.

| Field | Meaning |
| --- | --- |
| `id` | SHA-256 identity of the serialized event. |
| `source_chart_sha256`, `audio_sha256` | The chart and recording this event belongs to. |
| `parents` | Event IDs known when the action was created. |
| `actor` | The contributor's claimed kind and name. |
| `at_ms` | Action time for display and history. |
| `origin_revision` | The originating project's revision. |
| `operation` | Proposal, discussion, review, suggestion or merge action. |

The ID hashes the serde_json tuple
`(source_chart_sha256, audio_sha256, parents, actor, at_ms, origin_revision, operation)`.
Ordered collections make its serialization stable. Changing this representation
requires a format version change. Hashes detect changes; they are not signatures,
and actor names are not authenticated identities.

Imports retain original IDs and authors. Matching events coalesce. Invalid
hashes, missing parents, cycles, incompatible source files and conflicting event
contents are rejected. Partial folder transfers can be retried after all files
arrive. Limits are 2,000 events, 16 MiB per imported JSON file and 64 MiB per
directory scan.

## Applying changes

Imported reviews do not automatically edit notes. Merging validates the proposal,
its approval and the current notes. Conflicts require explicit choices and a new
proposal. Earlier approval does not transfer to changed content.

Project writes use the normal revision check, saved actions and checkpoints.
Keep a complete project copy for recovery. Share event files or bundles rather
than merging concurrent copies of `project.json`.

See `src/collaboration.rs`, `src/collaboration_exchange.rs` and `src/review_git.rs`
for the Rust types and transport rules. The format is independent of Git; a
shared directory or portable bundle can carry the same events.
