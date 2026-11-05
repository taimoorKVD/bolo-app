-- Bolo self-hosted API: persistence schema (Supabase project oahexfxwouuhplxdjxqf).
-- The Node service is the ONLY client and uses the service_role key server-side.
-- Stores account/usage METADATA only (hashed tokens, minutes, audit). Never audio or text.
-- Idempotent; safe to re-run.

create table if not exists public.bolo_state (
  id         text primary key check (id ~ '^[a-z0-9_-]{1,32}$'),
  data       jsonb not null check (jsonb_typeof(data) = 'object'),
  updated_at timestamptz not null default now()
);

-- Deny-by-default: RLS on (and forced for the table owner), no policies, so the
-- anon/authenticated (publishable) keys can never read or write it. service_role bypasses RLS.
alter table public.bolo_state enable row level security;
alter table public.bolo_state force row level security;
revoke all on table public.bolo_state from public, anon, authenticated;
grant select, insert, update on table public.bolo_state to service_role;

comment on table public.bolo_state is
  'Bolo API state snapshot (metadata only, no audio/text). Written only by the Plesk Node service via service_role.';
