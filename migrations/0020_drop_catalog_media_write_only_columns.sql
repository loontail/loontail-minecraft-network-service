-- CB-10: drop the three `catalog_media` columns only the now-deleted `attach_media`
-- endpoint ever wrote. No read path ever selected them: the public DTO reads
-- (client_id, role, url, width, height) and the admin list reads
-- (id, role, url, width, height, sort_order), and a contract test pins `formats`
-- being absent from the DTO. `formats` in particular was a serialised second table
-- (the Strapi-era thumbnail/large/medium/small map) that no code deserialised.
--
-- 0006's column comment still describes `formats` as the image formats map. It is
-- left untouched on purpose: `sqlx::migrate!` checksums every applied migration file,
-- so editing an already-applied one makes the next boot refuse to migrate. This file
-- is the correction of record.
--
-- Destructive: drops three columns. Only `POST /admin/catalog/clients/{id}/media`
-- ever populated them and it had no client in any repo, so rows written by the
-- bytes-upload endpoint (the only one the admin SPA calls) had all three NULL/'{}'.
ALTER TABLE catalog_media
    DROP COLUMN IF EXISTS formats,
    DROP COLUMN IF EXISTS name,
    DROP COLUMN IF EXISTS hash;
