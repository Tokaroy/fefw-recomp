# fefw-recomp Report Worker (Cloudflare Workers + R2 + D1)

Minimal serverless receiver for opt-in, anonymized crash and performance reports sent by the launcher.

- **Free tier**: Runs on a free `*.workers.dev` subdomain (no paid domain needed), 10 GB R2 storage, 5 GB D1 SQLite, 100k requests/day.
- **Privacy**: Client IP addresses are never stored. All user paths, hostnames, and Steam IDs are scrubbed by the launcher before upload.

## One-time setup

From `tools/report-worker/`:

1. **Create the R2 bucket** (and set a 90-day lifecycle expiration rule in the Cloudflare dashboard under R2 -> `fefw-reports` -> Settings -> Object lifecycle rules so storage stays well inside the 10 GB free tier):
   ```bash
   npx wrangler r2 bucket create fefw-reports
   ```

2. **Create the D1 database**:
   ```bash
   npx wrangler d1 create fefw-reports-index
   ```
   Copy the `database_id` from the output into `wrangler.toml` (replacing `REPLACE_WITH_D1_DATABASE_ID`).

3. **Apply the D1 schema**:
   ```bash
   npx wrangler d1 execute fefw-reports-index --remote --file=schema.sql
   ```

4. **Set an admin token** (used by `tools/fetch_reports.py` to list and download reports — generate any long random string, e.g. `openssl rand -hex 32`):
   ```bash
   npx wrangler secret put ADMIN_TOKEN
   ```

5. **Deploy**:
   ```bash
   npx wrangler deploy
   ```
   Wrangler prints your live URL, e.g. `https://fefw-reports.<your-subdomain>.workers.dev`.

## Wiring the URL into the launcher

Either:
- Set `FEFW_REPORT_URL=https://fefw-reports.<your-subdomain>.workers.dev` in your release/CI build environment (baked into the launcher binary at compile time), **or**
- Set `DEFAULT_URL` in `launcher/src-tauri/src/report.rs`.

For local testing, setting `FEFW_REPORT_URL` in the environment when running the launcher overrides the baked-in URL.

## Fetching reports locally

```bash
export FEFW_REPORT_URL="https://fefw-reports.<your-subdomain>.workers.dev"
export FEFW_REPORT_ADMIN_TOKEN="<your ADMIN_TOKEN>"

python3 tools/fetch_reports.py list
python3 tools/fetch_reports.py list --trigger crash --limit 20
python3 tools/fetch_reports.py get <report-id> --out work/reports
python3 tools/fetch_reports.py sync --limit 30 --out work/reports
```
