# Safety Contract

`contextpatch` exists to make agent edits small, anchored, atomic, and reviewable.

## Write rules

1. Do not overwrite whole files by default.
2. Require exact anchors, hashes, or patch context for persistent writes.
3. Refuse ambiguous replacements.
4. Write through a temporary file and atomic rename where supported.
5. Expose Git state before guarded edits.
6. Prefer previewable diffs over hidden mutation.
7. Never provide an unrestricted shell or recursive delete primitive.

## Default-deny tools

The server should not expose generic `write_file`, unrestricted `delete`, recursive directory writes, or shell execution as default tools.
