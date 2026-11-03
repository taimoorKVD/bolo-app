# Releasing Bolo

Releases are automated by `.github/workflows/release.yml`. It mirrors
`scripts/release.ps1` and `scripts/publish-store.ps1` but does **not** bump the version.

## How to ship

1. Bump the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`, then commit to `main`.
2. Make sure Partner Center has no pending submission for Bolo (9NMW6C99BF04).
3. Tag and push: `git tag -a v0.1.6 -m "What changed"` then `git push origin v0.1.6`.
   The tag message becomes the updater notes. The tag must equal the app version or the run stops.

## What the workflow does

| Job | Steps |
| --- | --- |
| build | `cargo test` (never `--ignored`), signed NSIS build (`createUpdaterArtifacts`), writes `latest.json` |
| website | Commits `downloads/Bolo_<v>_x64-setup.exe`, `updates/latest.json`, `version.json` and the `index.html` download links to `taimoorKVD/bolo-website`. That push runs the site's own Deploy workflow, which pulls it onto the server at bolo.thebetawebsite.com (same paths as 0.1.4). Then it waits until `/updates/latest.json` shows the new version. |
| store | Builds the MSIX with `scripts/build-msix.ps1` and submits it with the msstore CLI |
| github-release | Creates a GitHub Release with the installer, `.sig` and `latest.json` |

## Secrets (Settings > Secrets and variables > Actions on bolo-app)

| Name | What it is | Where the value comes from |
| --- | --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | Private key that signs updates so installed copies trust them | Full contents of `_secrets/bolo-updater.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for that key | The password used when the key was generated; leave the secret empty if it has none |
| `WEBSITE_REPO_DEPLOY_KEY` | SSH private key allowed to push to `taimoorKVD/bolo-website` | Generate a new key pair; add the `.pub` to bolo-website > Settings > Deploy keys with **write access**; paste the private half here |
| `MSSTORE_TENANT_ID` | Azure AD tenant of the Partner Center account | `TenantId` in `_secrets/store-api.json` |
| `MSSTORE_CLIENT_ID` | App registration used for Store submissions | `ClientId` in `_secrets/store-api.json` |
| `MSSTORE_CLIENT_SECRET` | Secret of that app registration | `ClientSecret` in `_secrets/store-api.json` (renew in Azure when it expires) |
| `MSSTORE_SELLER_ID` | Your Partner Center seller ID | `SellerId` in `_secrets/store-api.json` |
| `GITHUB_TOKEN` | Creates the GitHub Release | Built in, nothing to add |

The server itself keeps using the bolo-website repo's existing secrets (`DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_SSH_KEY`); nothing new is needed there.

## Notes

- The msstore CLI does not set per-listing release notes the way `publish-store.ps1` does. Add them in Partner Center if you want them.
- If the store job fails, the website and GitHub Release still go out. Re-run only the store job after clearing the pending submission.