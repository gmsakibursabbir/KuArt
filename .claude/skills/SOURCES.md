# Third-party skills

Installed 2026-10-09 at the user's request. Only SKILL.md files and their reference docs were copied:
no hooks, scripts or binaries. Each folder keeps its upstream LICENSE.

| Skill folder(s) | Source | Commit |
|---|---|---|
| vibe-security | https://github.com/raroque/vibe-security-skill | `850938f` |
| caveman, ultracave, megacave | https://github.com/JuliusBrussee/caveman | `2e08b91` |
| no-ai-slop | https://github.com/petergyang/no-ai-slop | `000650b` |
| ponytail, ponytail-audit, ponytail-debt, ponytail-gain, ponytail-help, ponytail-review | https://github.com/DietrichGebert/ponytail | `9cc65d0` |

Not installed: caveman's `caveman-compress` (runs bundled Python that calls Claude and overwrites files), `caveman-stats` and `cavecrew` (depend on plugin hooks / agent files), and all plugin hooks.

## Added 2026-10-09 (second batch)

| Skill folder(s) | Source | Commit |
|---|---|---|
| brandkit, brutalist-skill, gpt-tasteskill, image-to-code-skill, imagegen-frontend-mobile, imagegen-frontend-web, llms.txt, minimalist-skill, output-skill, redesign-skill, soft-skill, stitch-skill, taste-skill, taste-skill-v1 | https://github.com/Leonxlnx/taste-skill | `b482f7a` |
| impeccable | https://github.com/pbakaus/impeccable | `9dad388` |

Not installed from impeccable: `scripts/` (launcher that downloads and runs a native binary, browser JS) and its
`.claude/settings.json` hooks (run that launcher after every edit). The skill falls back to reading PRODUCT.md /
DESIGN.md directly; `live`, `generate`, `pin` and `hooks` commands will not work.
