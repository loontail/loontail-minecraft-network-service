-- Launcher catalog: clients, keywords, servers, their localized text, media, and
-- the join tables. Shapes are driven by the launcher's Strapi-compatible JSON
-- (see the design doc §5 CATALOG and the launcher `client.ts`/`strapi.ts`
-- contracts): camelCase wire fields, `{data,meta:{pagination}}` envelope, draft
-- filter via `published_at`, server-relative media URLs, nullable version cols.

-- ---------------------------------------------------------------------------
-- Clients: one launcher "build" entry. Locale-independent fields live here;
-- localized title/description/short_description live in catalog_client_locales.
-- ---------------------------------------------------------------------------
CREATE TABLE catalog_clients (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug              TEXT NOT NULL UNIQUE,
    available         BOOLEAN NOT NULL DEFAULT false,
    minecraft_version TEXT,
    forge_version     TEXT,
    fabric_version    TEXT,
    runtime_version   TEXT,
    bundle_slug       TEXT,
    sort_order        INT NOT NULL DEFAULT 0,
    published_at      TIMESTAMPTZ,                 -- NULL => draft (filtered out of public reads)
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX catalog_clients_published ON catalog_clients (published_at);

CREATE TABLE catalog_client_locales (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    client_id         UUID NOT NULL REFERENCES catalog_clients (id) ON DELETE CASCADE,
    locale            TEXT NOT NULL,
    title             TEXT NOT NULL,
    description       TEXT,
    short_description TEXT,
    UNIQUE (client_id, locale)
);

CREATE INDEX catalog_client_locales_client ON catalog_client_locales (client_id);

-- ---------------------------------------------------------------------------
-- Media: one row per image. `role` selects the launcher slot
-- (poster | background | titleImage | screenshot). `formats` is the Strapi
-- formats map (thumbnail/large/medium/small) stored verbatim as JSONB. `url` is
-- server-relative; the launcher absolutizes it.
-- ---------------------------------------------------------------------------
CREATE TABLE catalog_media (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    client_id    UUID NOT NULL REFERENCES catalog_clients (id) ON DELETE CASCADE,
    role         TEXT NOT NULL CHECK (role IN ('poster', 'background', 'titleImage', 'screenshot')),
    url          TEXT NOT NULL,
    ext          TEXT,
    name         TEXT,
    hash         TEXT,
    mime         TEXT,
    width        INT,
    height       INT,
    size         INT,
    formats      JSONB NOT NULL DEFAULT '{}'::jsonb,
    sort_order   INT NOT NULL DEFAULT 0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX catalog_media_client_role ON catalog_media (client_id, role);

-- ---------------------------------------------------------------------------
-- Keywords + their localized titles.
-- ---------------------------------------------------------------------------
CREATE TABLE catalog_keywords (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug         TEXT NOT NULL UNIQUE,
    published_at TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE catalog_keyword_locales (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    keyword_id UUID NOT NULL REFERENCES catalog_keywords (id) ON DELETE CASCADE,
    locale     TEXT NOT NULL,
    title      TEXT NOT NULL,
    UNIQUE (keyword_id, locale)
);

CREATE INDEX catalog_keyword_locales_keyword ON catalog_keyword_locales (keyword_id);

-- ---------------------------------------------------------------------------
-- Servers: name (optional) + address. No localized text.
-- ---------------------------------------------------------------------------
CREATE TABLE catalog_servers (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug         TEXT NOT NULL UNIQUE,
    name         TEXT,
    address      TEXT NOT NULL,
    published_at TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- Join tables.
-- ---------------------------------------------------------------------------
CREATE TABLE catalog_client_keywords (
    client_id  UUID NOT NULL REFERENCES catalog_clients (id) ON DELETE CASCADE,
    keyword_id UUID NOT NULL REFERENCES catalog_keywords (id) ON DELETE CASCADE,
    sort_order INT NOT NULL DEFAULT 0,
    PRIMARY KEY (client_id, keyword_id)
);

CREATE INDEX catalog_client_keywords_keyword ON catalog_client_keywords (keyword_id);

CREATE TABLE catalog_client_servers (
    client_id  UUID NOT NULL REFERENCES catalog_clients (id) ON DELETE CASCADE,
    server_id  UUID NOT NULL REFERENCES catalog_servers (id) ON DELETE CASCADE,
    sort_order INT NOT NULL DEFAULT 0,
    PRIMARY KEY (client_id, server_id)
);

CREATE INDEX catalog_client_servers_server ON catalog_client_servers (server_id);
