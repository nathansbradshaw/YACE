# Migrating existing song folders

The offline Rust utility `migrate-songs` converts an existing charting library
without rebuilding its saved projects. Close projects and generation workers in
the selected library first. A plan is read-only:

```sh
cargo run --locked --release --no-default-features --bin migrate-songs -- '/path/to/Charts'
```

Apply with a **new backup directory outside the library** on the same filesystem:

```sh
cargo run --locked --release --no-default-features --bin migrate-songs -- \
  '/path/to/Charts' --apply --backup '/path/to/Backups/library-migration-001'
```

Each replacement is prepared and checked separately before the original folder
is moved into the backup. On macOS, APFS clones avoid duplicating unchanged
audio blocks during preparation. No source folder is deleted. Existing shared
editing projects are backed up too. File hashes and per-item recovery journals
are retained alongside `report.json`.

Charts, song.ini, playable audio and artwork stay at the song root. Editing state
moves to `.yace/project`; complete legacy generation wrappers move beneath
`.yace/jobs/generation`. A new game export uses the **current saved project**,
including edits after generation. The previous game export remains in the job's
`legacy-game-song` folder and in the original backup.

Referenced stems are verified and retained with relative paths. Checkpoint hashes
are updated to preserve undo/redo contents after relocation. The immutable source
snapshot, review witnesses, revision and audit history are preserved. Legacy
separation-cache bytes move beneath their containing folder's `.yace/cache`, with
a relative compatibility link at the old cache path for historical witnesses.
Incomplete jobs, changed files, symlinks inside selected songs, conflicting
destinations and active kit auditions are rejected before replacement.

If a project recording was changed, repair is allowed only from that job's saved
input when its bytes match the project's original SHA-256 and import receipt.
Only the staged project is repaired; the changed recording remains in the exact
original-folder backup, and the repair is recorded in the migration journal.

Every migrated project is loaded again from its final path and through library
discovery. Saved chart content, settings, revision and undo/redo contents must
match the original. Completed jobs additionally verify their publication hashes.

To undo the migration, close affected projects and run:

```sh
cargo run --locked --release --no-default-features --bin migrate-songs -- \
  --restore '/path/to/Backups/library-migration-001'
```

Restoration verifies original backup hashes and retains migrated folders as
`migrated-before-restore` rather than overwriting them. Per-item journals also
support restoration if the final batch report was never written.

If one song stops a batch, finalize reports and legacy-cache links for the songs
that already completed, without retrying or changing the unfinished songs:

```sh
cargo run --locked --release --no-default-features --bin migrate-songs -- \
  --finalize '/path/to/Backups/library-migration-001'
```

The report lists both completed and remaining folders. A remaining song can then
be migrated separately into another new backup directory after its issue is fixed.
