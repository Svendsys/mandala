# How to file these issues on GitHub

Prerequisite: **Issues must be enabled** on the repository (Settings → General → Features → Issues). As of 2026-07-04 the API returned `410 Issues has been disabled` — everything below is ready to run the moment the box is checked.

## Procedure (for a coding agent or a human)

1. Read `MANIFEST.json`. File issues **in manifest order** (P0-01 → P3-53). For each entry:
   - **Title**: the draft's H1 text (keep the `P0-01:`-style prefix — it encodes priority and maps back to this directory).
   - **Body**: the draft file's content minus the H1 line, with two additions:
     - a `## Dependencies` section generated from the manifest fields, substituting already-filed issue numbers for P-ids (`depends_on` → "Blocked by #N"; `blocks`/`blocks_soft` → "Blocks #N (soft)"; `soft_after` → "Prefer after #N"; `coordinate_with` → "Coordinate with #N — same files/decision"). For forward references (target not yet filed), write the P-id and title; a second pass (step 3) replaces them.
     - a footer: `*Part of the July 2026 comprehensive quality review — [README](<branch URL>/README.md) · evidence: [findings/](<branch URL>/findings/)*` using the branch-qualified URL `https://github.com/Svendsys/mandala/blob/claude/repo-quality-review-qm6p15/reviews/2026-07-comprehensive-quality-review/` (update if the branch merges — root-relative links work from main after merge).
   - **Label**: the manifest's `label` (only default labels are used: `bug`, `enhancement`, `documentation`).
   - Record the returned issue number against the P-id.
2. After all 53 are filed, file **EPIC-00** from `issues/EPIC-00-tracking.md`, replacing every P-id in its checklists with `#N` (GitHub then renders live progress). Label: `enhancement`.
3. Second pass: update the handful of issues whose Dependencies sections contained forward P-id references (the manifest's `blocks`/`blocks_soft` sources: P1-12, P0-06, P2-38, P2-43, P1-23, P1-25, P1-26, P1-17, P2-47) so they carry real numbers.
4. Optionally, attach the four **hard-blocker** pairs via GitHub's native sub-issue/relationship UI; the body text is the source of truth either way.

## Notes

- Do not reflow or "improve" the draft bodies — file:line references are load-bearing and were verified against commit `59cd115`.
- If significant time has passed since the review commit, re-verify line numbers before starting work on any issue (the drafts state file + function names, which survive drift better than line numbers).
- The four human-decision gates (P2-45, P2-47-C, P1-15, P2-43-C) should be answered by the maintainer in an issue comment before an agent picks them up.
