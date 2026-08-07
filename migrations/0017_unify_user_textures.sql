-- CB-1: `skins` and `capes` were column-for-column identical (capes merely lacked
-- `variant`), which forced the table name to be interpolated into 7 queries and the
-- upsert to be written twice. Collapse both into `user_textures` with a `kind`
-- discriminator so the kind binds as a query parameter.
--
-- Destructive: drops `skins` and `capes`. Existing rows are copied first, so no
-- texture data is lost; the migration is safe on a populated database.
CREATE TABLE user_textures (
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind         TEXT NOT NULL CHECK (kind IN ('skin', 'cape')),
    profile_uuid TEXT NOT NULL,
    username     TEXT NOT NULL,
    file_path    TEXT NOT NULL,
    file_url     TEXT NOT NULL,
    file_size    INT NOT NULL DEFAULT 0,
    variant      TEXT CHECK (variant IN ('CLASSIC', 'SLIM')),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, kind),
    -- Only skins have a model variant; capes must not carry one. This is the
    -- constraint the two-table shape could not express, and the reason `capes`
    -- silently drifted without a `variant` column.
    CONSTRAINT user_textures_variant_kind CHECK ((kind = 'skin') = (variant IS NOT NULL))
);

INSERT INTO user_textures
    (user_id, kind, profile_uuid, username, file_path, file_url, file_size, variant, updated_at)
SELECT user_id, 'skin', profile_uuid, username, file_path, file_url, file_size, variant, updated_at
FROM skins;

INSERT INTO user_textures
    (user_id, kind, profile_uuid, username, file_path, file_url, file_size, variant, updated_at)
SELECT user_id, 'cape', profile_uuid, username, file_path, file_url, file_size, NULL, updated_at
FROM capes;

DROP TABLE skins;
DROP TABLE capes;
