---
name: setup-skills
description: Set up this repo for the agent skills
disable-model-invocation: true
---
# Setup Skills

## Labels

Ensure these labels exist on the current repo (`gh label list`, or a GitHub MCP server). Create any that are missing, leave existing ones untouched:

    gh label create needs-vision-align --color FBCA04 --description "Spec is missing the vision only the human holds; needs a /vision-align session"
    gh label create ready-for-agent --color 0E8A16 --description "Self-contained spec; an agent can implement it without further context"

Report which were created and which already existed.

**No `gh` — a GitHub MCP server is the only GitHub access?** If the server exposes a label-writing operation, use it — it sets colour and description directly. Where it doesn't (some servers only read labels), you still don't need a separate creation step: GitHub **auto-creates a label the first time it is applied to an issue**, so applying it while creating or updating any issue brings it into existence repo-wide (to seed one without leaving it on the wrong issue, apply then unapply — the label persists). The only catch is cosmetic — an auto-created label gets GitHub's default grey colour and an empty description — so set them with `gh label edit <name> --color <hex> --description "…"` (values above) once `gh` is available. A functional label beats a blocked workflow; never stall waiting for one to be pre-made.

## gh version

Native issue dependency links (`--blocked-by`, `--parent`) need `gh` ≥ 2.94.0. Check `gh --version`; if it's older, report that they're unavailable until it's upgraded.