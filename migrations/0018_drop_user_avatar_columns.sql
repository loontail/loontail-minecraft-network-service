-- NB-4: drop the vestigial `users.avatar_url` and `users.skin_hash`. Added in
-- migration 0001 and read into three wire DTOs, but never written by any code path,
-- so both were NULL for every user. In-game avatars come from the vanilla skin
-- provider keyed on `minecraft_uuid`; `skin_hash` was never consulted by any client.
-- Same shape as the 0014 precedent.
--
-- Destructive: drops two columns. Every value in them was NULL, so no data is lost.
ALTER TABLE users
    DROP COLUMN IF EXISTS avatar_url,
    DROP COLUMN IF EXISTS skin_hash;
