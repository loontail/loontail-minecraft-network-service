//! Admin catalog CRUD (mounted at `/admin/catalog`, `AdminUser`-guarded). These
//! are deliberately minimal but functional: create/update/delete clients,
//! keywords, and servers; publish/unpublish; and a basic media-attach. Admin
//! reads return rows directly (drafts included) rather than the public Strapi
//! envelope.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use loontail_core::auth::AdminUser;
use loontail_core::error::{AppError, AppResult};
use loontail_core::AppState;

fn unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.is_unique_violation())
}

// --- Clients ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertClient {
    pub slug: String,
    #[serde(default)]
    pub available: bool,
    pub minecraft_version: Option<String>,
    pub forge_version: Option<String>,
    pub fabric_version: Option<String>,
    pub runtime_version: Option<String>,
    pub bundle_slug: Option<String>,
    #[serde(default)]
    pub sort_order: i32,
    /// Localized text rows to (re)write for this client.
    #[serde(default)]
    pub locales: Vec<ClientLocaleInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientLocaleInput {
    pub locale: String,
    pub title: String,
    pub description: Option<String>,
    pub short_description: Option<String>,
}

pub async fn create_client(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<UpsertClient>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let mut tx = state.pool.begin().await?;
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO catalog_clients \
         (slug, available, minecraft_version, forge_version, fabric_version, \
          runtime_version, bundle_slug, sort_order) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id",
    )
    .bind(&body.slug)
    .bind(body.available)
    .bind(&body.minecraft_version)
    .bind(&body.forge_version)
    .bind(&body.fabric_version)
    .bind(&body.runtime_version)
    .bind(&body.bundle_slug)
    .bind(body.sort_order)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if unique_violation(&e) {
            AppError::Conflict("client slug already exists".into())
        } else {
            e.into()
        }
    })?;

    write_client_locales(&mut tx, id, &body.locales).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub async fn update_client(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpsertClient>,
) -> AppResult<Json<Value>> {
    let mut tx = state.pool.begin().await?;
    let affected = sqlx::query(
        "UPDATE catalog_clients SET slug=$2, available=$3, minecraft_version=$4, \
         forge_version=$5, fabric_version=$6, runtime_version=$7, bundle_slug=$8, \
         sort_order=$9, updated_at=now() WHERE id=$1",
    )
    .bind(id)
    .bind(&body.slug)
    .bind(body.available)
    .bind(&body.minecraft_version)
    .bind(&body.forge_version)
    .bind(&body.fabric_version)
    .bind(&body.runtime_version)
    .bind(&body.bundle_slug)
    .bind(body.sort_order)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        if unique_violation(&e) {
            AppError::Conflict("client slug already exists".into())
        } else {
            e.into()
        }
    })?
    .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound("client not found".into()));
    }
    if !body.locales.is_empty() {
        sqlx::query("DELETE FROM catalog_client_locales WHERE client_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        write_client_locales(&mut tx, id, &body.locales).await?;
    }
    tx.commit().await?;
    Ok(Json(json!({ "id": id })))
}

async fn write_client_locales(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    client_id: Uuid,
    locales: &[ClientLocaleInput],
) -> AppResult<()> {
    for l in locales {
        sqlx::query(
            "INSERT INTO catalog_client_locales \
             (client_id, locale, title, description, short_description) \
             VALUES ($1,$2,$3,$4,$5) \
             ON CONFLICT (client_id, locale) DO UPDATE SET \
             title=EXCLUDED.title, description=EXCLUDED.description, \
             short_description=EXCLUDED.short_description",
        )
        .bind(client_id)
        .bind(&l.locale)
        .bind(&l.title)
        .bind(&l.description)
        .bind(&l.short_description)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub async fn delete_client(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let affected = sqlx::query("DELETE FROM catalog_clients WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound("client not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn publish_client(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    set_published(&state, "catalog_clients", id, true).await
}

pub async fn unpublish_client(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    set_published(&state, "catalog_clients", id, false).await
}

async fn set_published(
    state: &AppState,
    table: &str,
    id: Uuid,
    published: bool,
) -> AppResult<Json<Value>> {
    // `table` is a fixed internal literal (never user input), so interpolating it
    // is safe here.
    let setter = if published {
        "published_at = now()"
    } else {
        "published_at = NULL"
    };
    let affected = sqlx::query(&format!(
        "UPDATE {table} SET {setter}, updated_at = now() WHERE id = $1"
    ))
    .bind(id)
    .execute(&state.pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound("not found".into()));
    }
    Ok(Json(json!({ "id": id, "published": published })))
}

// --- Media attach ----------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachMedia {
    pub role: String,
    pub url: String,
    pub ext: Option<String>,
    pub name: Option<String>,
    pub hash: Option<String>,
    pub mime: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub size: Option<i32>,
    #[serde(default = "default_formats")]
    pub formats: Value,
    #[serde(default)]
    pub sort_order: i32,
}

fn default_formats() -> Value {
    json!({})
}

const MEDIA_ROLES: [&str; 4] = ["poster", "background", "titleImage", "screenshot"];

pub async fn attach_media(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(client_id): Path<Uuid>,
    Json(body): Json<AttachMedia>,
) -> AppResult<(StatusCode, Json<Value>)> {
    if !MEDIA_ROLES.contains(&body.role.as_str()) {
        return Err(AppError::BadRequest(format!(
            "invalid media role '{}'",
            body.role
        )));
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM catalog_clients WHERE id=$1)")
            .bind(client_id)
            .fetch_one(&state.pool)
            .await?;
    if !exists {
        return Err(AppError::NotFound("client not found".into()));
    }
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO catalog_media \
         (client_id, role, url, ext, name, hash, mime, width, height, size, formats, sort_order) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) RETURNING id",
    )
    .bind(client_id)
    .bind(&body.role)
    .bind(&body.url)
    .bind(&body.ext)
    .bind(&body.name)
    .bind(&body.hash)
    .bind(&body.mime)
    .bind(body.width)
    .bind(body.height)
    .bind(body.size)
    .bind(&body.formats)
    .bind(body.sort_order)
    .fetch_one(&state.pool)
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

// --- Keywords --------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertKeyword {
    pub slug: String,
    #[serde(default)]
    pub locales: Vec<KeywordLocaleInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeywordLocaleInput {
    pub locale: String,
    pub title: String,
}

pub async fn create_keyword(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<UpsertKeyword>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let mut tx = state.pool.begin().await?;
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO catalog_keywords (slug) VALUES ($1) RETURNING id",
    )
    .bind(&body.slug)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if unique_violation(&e) {
            AppError::Conflict("keyword slug already exists".into())
        } else {
            e.into()
        }
    })?;
    for l in &body.locales {
        sqlx::query(
            "INSERT INTO catalog_keyword_locales (keyword_id, locale, title) \
             VALUES ($1,$2,$3) ON CONFLICT (keyword_id, locale) \
             DO UPDATE SET title = EXCLUDED.title",
        )
        .bind(id)
        .bind(&l.locale)
        .bind(&l.title)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub async fn delete_keyword(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let affected = sqlx::query("DELETE FROM catalog_keywords WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound("keyword not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn publish_keyword(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    set_published(&state, "catalog_keywords", id, true).await
}

pub async fn unpublish_keyword(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    set_published(&state, "catalog_keywords", id, false).await
}

// --- Servers ---------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertServer {
    pub slug: String,
    pub name: Option<String>,
    pub address: String,
}

pub async fn create_server(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<UpsertServer>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO catalog_servers (slug, name, address) VALUES ($1,$2,$3) RETURNING id",
    )
    .bind(&body.slug)
    .bind(&body.name)
    .bind(&body.address)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| {
        if unique_violation(&e) {
            AppError::Conflict("server slug already exists".into())
        } else {
            e.into()
        }
    })?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub async fn update_server(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpsertServer>,
) -> AppResult<Json<Value>> {
    let affected = sqlx::query(
        "UPDATE catalog_servers SET slug=$2, name=$3, address=$4, updated_at=now() WHERE id=$1",
    )
    .bind(id)
    .bind(&body.slug)
    .bind(&body.name)
    .bind(&body.address)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        if unique_violation(&e) {
            AppError::Conflict("server slug already exists".into())
        } else {
            e.into()
        }
    })?
    .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound("server not found".into()));
    }
    Ok(Json(json!({ "id": id })))
}

pub async fn delete_server(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    let affected = sqlx::query("DELETE FROM catalog_servers WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound("server not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn publish_server(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    set_published(&state, "catalog_servers", id, true).await
}

pub async fn unpublish_server(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Value>> {
    set_published(&state, "catalog_servers", id, false).await
}

// --- Relations -------------------------------------------------------------

pub async fn attach_keyword(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((client_id, keyword_id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    sqlx::query(
        "INSERT INTO catalog_client_keywords (client_id, keyword_id) VALUES ($1,$2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(client_id)
    .bind(keyword_id)
    .execute(&state.pool)
    .await
    .map_err(map_fk)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn attach_server(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((client_id, server_id)): Path<(Uuid, Uuid)>,
) -> AppResult<StatusCode> {
    sqlx::query(
        "INSERT INTO catalog_client_servers (client_id, server_id) VALUES ($1,$2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(client_id)
    .bind(server_id)
    .execute(&state.pool)
    .await
    .map_err(map_fk)?;
    Ok(StatusCode::NO_CONTENT)
}

fn map_fk(err: sqlx::Error) -> AppError {
    if matches!(&err, sqlx::Error::Database(db) if db.is_foreign_key_violation()) {
        AppError::NotFound("client, keyword, or server not found".into())
    } else {
        err.into()
    }
}
