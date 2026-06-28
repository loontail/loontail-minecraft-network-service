-- QUAL-1: drop the vestigial `users.yggdrasil_validated_at` column. Added in
-- migration 0003 but never read or written anywhere in the codebase (the `User`
-- mirror carried it only for `SELECT *` completeness). Removing both the column and
-- the mirrored field eliminates the dead schema/field pair.
ALTER TABLE users
    DROP COLUMN IF EXISTS yggdrasil_validated_at;
