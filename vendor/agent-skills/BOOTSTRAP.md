# Bootstrap: get agent-skills into your repo

A stepped guide to vendor these skills into a **downstream repository** and keep
them in sync. The sync tool ([`bin/agent-skills-sync`](bin/agent-skills-sync))
copies the skills in as real, committed files, so a plain `git clone` of your
repo already has them — nothing to fetch at runtime.

You do this once (steps 1–4); after that, syncing is a single command (step 5).

## What you end up with

```
your-repo/
├── agent-skills.config              # which skills you want (yours to edit; default: all)
├── vendor/agent-skills/             # vendored from upstream — managed by the sync tool
│   ├── bin/agent-skills-sync        # the sync tool itself (kept up to date by syncing)
│   ├── BOOTSTRAP.md
│   └── skills/<skill>/SKILL.md      # one dir per selected skill
├── .claude/skills/<skill> -> ../../vendor/agent-skills/skills/<skill>   # if you use Claude Code
└── .agents/skills/<skill> -> ../../vendor/agent-skills/skills/<skill>   # if you use Codex
```

Everything under `vendor/agent-skills/` is upstream-owned and rewritten on each
sync. Your `agent-skills.config` is the one file the tool seeds once and then
never touches.

## Prerequisites

- `git` and `bash` on your PATH.
- A downstream git repository to vendor into (run the commands from its root).

## Step 1 — Get the sync tool

Clone upstream once to a scratch location and run its sync tool against your repo.
The tool vendors *itself* on the first run, so you only need the clone this once:

```bash
git clone --depth 1 https://github.com/Svendsys/agent-skills.git /tmp/agent-skills-src

cd /path/to/your-repo            # the downstream repo you're vendoring into
/tmp/agent-skills-src/bin/agent-skills-sync
```

That first run:
- creates `agent-skills.config` (defaulting to **all** skills),
- copies the selected skills into `vendor/agent-skills/skills/`,
- vendors the sync tool to `vendor/agent-skills/bin/agent-skills-sync`.

You can delete `/tmp/agent-skills-src` afterwards.

## Step 2 — Choose your skills

Open `agent-skills.config` and keep the skills you want. The default `*` means
"all"; to pin a subset, list names instead:

```
pr-review
address-pr-comments
```

Re-run the sync to apply your choice — skills you removed from the config are
deleted from `vendor/agent-skills/skills/` on the next run:

```bash
./vendor/agent-skills/bin/agent-skills-sync
```

## Step 3 — Wire discovery for your tools

Create the skills dir for each tool you use; the sync tool fills it with relative
symlinks (and prunes them when you drop a skill) on every run:

```bash
mkdir -p .claude/skills      # Claude Code looks here
mkdir -p .agents/skills      # Codex looks here
./vendor/agent-skills/bin/agent-skills-sync
```

Only the dirs that exist are wired, so create just the ones you need. If your
sandbox can't follow symlinked skill dirs, point your tool at
`vendor/agent-skills/skills/` directly instead.

## Step 4 — Commit

```bash
git add agent-skills.config vendor/agent-skills .claude/skills .agents/skills
git commit -m "Vendor agent-skills"
```

The skills now ride along with your repo.

## Step 5 — Sync whenever you want updates

```bash
./vendor/agent-skills/bin/agent-skills-sync
git add -A && git commit -m "Sync agent-skills"
```

Each sync pulls the latest skills, applies your config, and updates the sync tool
itself. Preview first with `--dry-run`:

```bash
./vendor/agent-skills/bin/agent-skills-sync --dry-run
```

## Options

`agent-skills-sync` reads these flags (each also has an env var):

| Flag | Env | Default |
| --- | --- | --- |
| `--repo URL` | `AGENT_SKILLS_REPO` | `https://github.com/Svendsys/agent-skills.git` |
| `--ref REF` | `AGENT_SKILLS_REF` | `master` |
| `--prefix DIR` | `AGENT_SKILLS_PREFIX` | `vendor/agent-skills` |
| `--config PATH` | `AGENT_SKILLS_CONFIG` | `agent-skills.config` |
| `--dry-run` | — | off |

Updates flow one way: edit skills upstream, then sync. Don't edit the vendored
copies — the next sync overwrites them.
