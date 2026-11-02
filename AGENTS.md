# Preview servers

- Use only 3030 (primary) or 3032 (secondary) for YACE previews across all chats,
  agents and worktrees. Inspect listeners and their worktree owners first.
- Reuse the appropriate existing server, coordinate a handoff, or wait. Never
  increment to another port, choose a random preview port, or take over an active
  allowed-port server.
- Use `yace-static-web serve 3030 --check` to inspect the selected source and
  served directory without starting a server.
- Track every preview server started for a task. Stop owned servers after
  verification or before switching away and verify their listeners are gone.
- Preserve source changes, saved projects, running native applications and
  active owners of allowed-port previews. Native IPC listeners and self-cleaning
  short-lived test fixtures are separate from persistent preview servers.
