-- Skin/cape registry. One skin and one cape per user (user_id UNIQUE).
-- file_path is the on-disk relative path; file_url is the server-relative URL the
-- launcher/textures handler serves and (for skins) the signed value references.
CREATE TABLE skins (
    user_id      UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    profile_uuid TEXT NOT NULL,
    username     TEXT NOT NULL,
    file_path    TEXT NOT NULL,
    file_url     TEXT NOT NULL,
    file_size    INT NOT NULL DEFAULT 0,
    variant      TEXT NOT NULL DEFAULT 'CLASSIC' CHECK (variant IN ('CLASSIC', 'SLIM')),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE capes (
    user_id      UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    profile_uuid TEXT NOT NULL,
    username     TEXT NOT NULL,
    file_path    TEXT NOT NULL,
    file_url     TEXT NOT NULL,
    file_size    INT NOT NULL DEFAULT 0,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
