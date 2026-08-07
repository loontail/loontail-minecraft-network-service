-- Bundle registry: a bundle is a named set of overlay files (mods/configs/etc.).
-- bundle_artifacts are the per-file rows the manifest is generated from.
CREATE TABLE bundles (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug              TEXT NOT NULL UNIQUE,
    name              TEXT NOT NULL,
    description       TEXT,
    version           TEXT,
    status            TEXT NOT NULL DEFAULT 'draft',   -- draft | processing | ready | failed
    files_count       INT NOT NULL DEFAULT 0,
    total_size        BIGINT NOT NULL DEFAULT 0,
    processing_error  TEXT,
    last_generated_at TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE bundle_artifacts (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bundle_id        UUID NOT NULL REFERENCES bundles (id) ON DELETE CASCADE,
    relative_path    TEXT NOT NULL,
    name             TEXT NOT NULL,
    category         TEXT NOT NULL,
    size             BIGINT NOT NULL DEFAULT 0,
    sha256           TEXT,
    is_dir           BOOLEAN NOT NULL DEFAULT false,
    download_once    BOOLEAN NOT NULL DEFAULT false,
    file_modified_at TIMESTAMPTZ
);

CREATE INDEX bundle_artifacts_bundle ON bundle_artifacts (bundle_id);
