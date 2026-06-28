-- Link each catalog client (build) to its owned bundle via a real FK. The legacy
-- `bundle_slug` text column stays (the launcher contract keeps `bundleSlug`); this
-- adds the authoritative `bundle_id` reference and backfills it from the slug.
ALTER TABLE catalog_clients
    ADD COLUMN bundle_id UUID REFERENCES bundles (id) ON DELETE SET NULL;

-- Backfill from the existing text slug where a matching bundle exists (tolerate
-- dangling/whitespace slugs: only set rows that map to a real bundle).
UPDATE catalog_clients c
SET bundle_id = b.id
FROM bundles b
WHERE c.bundle_slug IS NOT NULL
  AND btrim(c.bundle_slug) = b.slug
  AND c.bundle_id IS NULL;

-- Close the racy SELECT-then-INSERT upsert gap on bundle artifacts: a single
-- artifact path per bundle is now enforced at the DB level (was relying on an
-- application-level check). Fails if duplicate (bundle_id, relative_path) rows
-- already exist; acceptable for a fresh dev DB.
CREATE UNIQUE INDEX IF NOT EXISTS bundle_artifacts_bundle_path_uq
    ON bundle_artifacts (bundle_id, relative_path);
