# Releasing (escpos_viewer)

Este repo usa **release-plz** para automatizar versiones + tags.

## Flujo

1. Hacés commits a `main` (idealmente Conventional Commits: `feat:`, `fix:`, etc.).
2. GitHub Actions ([release-plz.yml](.github/workflows/release-plz.yml)) crea/actualiza un **Release PR** con:
   - bump de versión en `Cargo.toml`
   - actualización de `CHANGELOG.md`
3. Cuando mergeás el Release PR, release-plz crea el tag `vX.Y.Z`.
4. Ese tag dispara **tres builds en paralelo**:

| Plataforma | Workflow | Artefactos |
|------------|----------|------------|
| Windows | [windows-release.yml](.github/workflows/windows-release.yml) | `.exe` + `InstaladorVisorESCPOS.exe` (Inno Setup) |
| Linux (binary) | [release-linux.yml](.github/workflows/release-linux.yml) | `escpos_viewer` + `.tar.gz` con docs |
| Linux (CI previa) | [ci-linux.yml](.github/workflows/ci-linux.yml) | Clippy, tests, build de verificación |

## Requisitos en GitHub

- **Workflow permissions**: Settings → Actions → General → **Read and write permissions** + permitir crear PRs.
- **Secret `RELEASE_PLZ_TOKEN`** (recomendado): Fine-grained PAT con permisos `Contents: Read and write` y `Pull requests: Read and write`.
  Los tags creados con `GITHUB_TOKEN` no disparan otros workflows; el PAT soluciona eso.

## Auto-merge (opcional)

El workflow [auto-merge-release-plz.yml](.github/workflows/auto-merge-release-plz.yml) permite auto-merge para PRs de release-plz (ramas `release-plz-*`).

Para activarlo: Settings → General → Pull Requests → **Allow auto-merge**.

## Notas

- El instalador Windows toma la versión desde el tag (CI pasa `/DMyAppVersion=...` a Inno Setup). No edites `setup.iss` manualmente.
- Los workflows Linux instalan `libgtk-3-dev` y `libxdo-dev` automáticamente.
- Si no querés Release PR por cada commit, configura `release_commits` en `release-plz.toml`.
