# Releasing Bolo

Releases are automated by `.github/workflows/release.yml`. It mirrors
`scripts/release.ps1` but does **not** bump the version.

## How to ship

1. Bump the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `package.json`, and `package-lock.json` (the root package), then commit to `main`. Keep `productName` as `Bolo.com`: that string is the NSIS install folder and uninstall key, and changing it installs a second copy next to 0.1.6 instead of upgrading it. The exe name is `bolo.exe` (`mainBinaryName`). Do not change the Microsoft Store Identity Name or Publisher.
2. Tag and push: `git tag -a v0.1.6 -m "What changed"` then `git push origin v0.1.6`.
   The tag message becomes the updater notes. The tag must equal the app version or the run stops.
3. When the run is green, download `Bolo_<v>.0_x64.msixupload` from the GitHub Release and submit it in
   Partner Center (Bolo, 9NMW6C99BF04) by hand.

## Dry run (safe test)

Actions > Release > Run workflow, leave **dry_run** ticked. It runs tests, builds and signs the
installer, checks the signature against the app's updater public key, writes `latest.json` and builds
the `.msixupload`, all as run artifacts. It does not push the website and does not create a GitHub
Release, so users get nothing. A manual run that is not on a `v*` tag is always a dry run.

## What the workflow does

| Job | Steps |
| --- | --- |
| build | `cargo test` (never `--ignored`), NSIS build, signs it with `tauri signer sign -p ""` through cmd with stdin closed (an empty password can't hang), verifies the key id, writes `latest.json` |
| store | Builds the MSIX with `scripts/build-msix.ps1` (no in-app updater) and wraps it as `.msixupload`. Automatic Store submission is a commented block; uncomment it once the `MSSTORE_*` secrets exist |
| website | Real releases only. Commits the installer, `updates/latest.json`, `version.json` and the `index.html` download links to `taimoorKVD/bolo-website`. That push runs the site's own Deploy workflow, which pulls it onto the server. Then it waits until `/updates/latest.json` shows the new version |
| github-release | Real releases only. Creates a GitHub Release with the installer, `.sig`, `latest.json` and `.msixupload` |

`windows-installer.yml` no longer builds on tags, so a release builds once.

## Secrets (bolo-app > Settings > Secrets and variables > Actions)

| Name | What it is | Where the value comes from |
| --- | --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | Private key that signs updates so installed copies trust them | Contents of `_secrets/bolo-updater.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for that key | Empty: the key has no password (the workflow always passes `-p ""`) |
| `WEBSITE_REPO_DEPLOY_KEY` | SSH key allowed to push to `taimoorKVD/bolo-website` | Private half of `_secrets/bolo-website-deploy`; the `.pub` is a write deploy key on bolo-website named "bolo-app release workflow" |
| `MSSTORE_TENANT_ID`, `MSSTORE_CLIENT_ID`, `MSSTORE_CLIENT_SECRET`, `MSSTORE_SELLER_ID` | Optional, for automatic Store submission | `_secrets/store-api.json` (needs Partner Center API access); not set today |
| `GITHUB_TOKEN` | Creates the GitHub Release | Built in |

The server keeps using bolo-website's own secrets (`DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY`).