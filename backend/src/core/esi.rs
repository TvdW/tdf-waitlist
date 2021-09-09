use serde::{Deserialize, Serialize};
use std::sync::Arc;

struct ESIRawClient {
    http: reqwest::Client,
    client_id: String,
    client_secret: String,
}

pub struct ESIClient {
    db: Arc<crate::DB>,
    raw: ESIRawClient,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

#[derive(Debug)]
pub struct AuthResult {
    pub character_id: i64,
    pub character_name: String,
    pub access_token: String,
    pub access_token_expiry: chrono::DateTime<chrono::Utc>,
    pub refresh_token: String,
    pub scopes: Vec<String>,
}

#[derive(Debug)]
pub enum ESIError {
    DatabaseError(sqlx::Error),
    HTTPError(reqwest::Error),
    NoToken,
    NotFound,
    Forbidden,
    HTTP520,
    MissingScope,
    InvalidToken,
}

#[derive(Debug, Deserialize)]
struct JWTToken {
    #[serde(rename = "sub")]
    subject: String,
    #[serde(rename = "scp")]
    scopes: Vec<String>,
    name: String,
    // Yes, the token also contains 'exp'. But we get this from the oauth token endpoint already,
    // and we should probably stick to that.
}

struct ParsedToken {
    character_id: i64,
    name: String,
    scopes: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
#[allow(non_camel_case_types)]
pub enum ESIScope {
    PublicData,
    Fleets_ReadFleet_v1,
    Fleets_WriteFleet_v1,
    UI_OpenWindow_v1,
    Skills_ReadSkills_v1,
    Clones_ReadImplants_v1,
}

impl ESIScope {
    pub fn as_str(&self) -> &'static str {
        use ESIScope::*;
        match self {
            PublicData => "publicData",
            Fleets_ReadFleet_v1 => "esi-fleets.read_fleet.v1",
            Fleets_WriteFleet_v1 => "esi-fleets.write_fleet.v1",
            UI_OpenWindow_v1 => "esi-ui.open_window.v1",
            Skills_ReadSkills_v1 => "esi-skills.read_skills.v1",
            Clones_ReadImplants_v1 => "esi-clones.read_implants.v1",
        }
    }
}

impl From<sqlx::Error> for ESIError {
    fn from(error: sqlx::Error) -> Self {
        ESIError::DatabaseError(error)
    }
}

impl From<reqwest::Error> for ESIError {
    fn from(error: reqwest::Error) -> Self {
        if error.is_status() {
            match error.status().unwrap().as_u16() {
                403 => return ESIError::Forbidden,
                404 => return ESIError::NotFound,
                520 => return ESIError::HTTP520, // XXX These have a body!
                _ => (),
            };
        }
        ESIError::HTTPError(error)
    }
}

impl ESIRawClient {
    pub fn new(client_id: String, client_secret: String) -> ESIRawClient {
        ESIRawClient {
            http: reqwest::Client::builder()
                .user_agent("Waitlist (https://github.com/TvdW/tdf-waitlist)")
                .build()
                .unwrap(),
            client_id,
            client_secret,
        }
    }

    async fn process_oauth_token(
        &self,
        grant_type: &str,
        token: &str,
    ) -> Result<OAuthTokenResponse, reqwest::Error> {
        #[derive(Serialize)]
        struct OAuthTokenRequest<'a> {
            grant_type: &'a str,
            refresh_token: Option<&'a str>,
            code: Option<&'a str>,
        }

        let request = OAuthTokenRequest {
            grant_type,
            refresh_token: match grant_type {
                "refresh_token" => Some(token),
                _ => None,
            },
            code: match grant_type {
                "refresh_token" => None,
                _ => Some(token),
            },
        };
        self.http
            .post("https://login.eveonline.com/v2/oauth/token")
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&request)
            .send()
            .await?
            .error_for_status()?
            .json::<OAuthTokenResponse>()
            .await
    }

    fn parse_token(&self, access_token: &str) -> Result<ParsedToken, ESIError> {
        // Look, this is "dangerous". But really all we're doing here is decoding a
        // base64-encoded JSON payload that was given to us over TLS from a trusted source.
        // It would indeed be dangerous for the other end to accept an unverified
        // access token, but for us? Nah.
        let decoded = match jsonwebtoken::dangerous_insecure_decode::<JWTToken>(access_token) {
            Ok(dec) => dec,
            Err(_) => return Err(ESIError::InvalidToken),
        };

        let subj_parts = decoded.claims.subject.split(":").collect::<Vec<_>>();
        if subj_parts.len() != 3 {
            return Err(ESIError::InvalidToken);
        }
        let character_id: i64 = match subj_parts[2].parse() {
            Ok(id) => id,
            Err(_) => return Err(ESIError::InvalidToken),
        };

        Ok(ParsedToken {
            character_id,
            name: decoded.claims.name,
            scopes: decoded.claims.scopes,
        })
    }

    pub async fn process_auth(
        &self,
        grant_type: &str,
        token: &str,
    ) -> Result<AuthResult, ESIError> {
        let token = self.process_oauth_token(grant_type, token).await?;
        let parsed = self.parse_token(&token.access_token)?;
        Ok(AuthResult {
            character_id: parsed.character_id,
            character_name: parsed.name,
            access_token: token.access_token,
            access_token_expiry: chrono::Utc::now()
                + chrono::Duration::seconds(token.expires_in / 2),
            refresh_token: token.refresh_token,
            scopes: parsed.scopes,
        })
    }

    pub async fn get(
        &self,
        url: &str,
        access_token: &str,
    ) -> Result<reqwest::Response, reqwest::Error> {
        Ok(self
            .http
            .get(url)
            .bearer_auth(access_token)
            .send()
            .await?
            .error_for_status()?)
    }

    pub async fn delete(
        &self,
        url: &str,
        access_token: &str,
    ) -> Result<reqwest::Response, reqwest::Error> {
        Ok(self
            .http
            .delete(url)
            .bearer_auth(access_token)
            .send()
            .await?
            .error_for_status()?)
    }

    pub async fn post<E: Serialize + ?Sized>(
        &self,
        url: &str,
        input: &E,
        access_token: &str,
    ) -> Result<reqwest::Response, reqwest::Error> {
        Ok(self
            .http
            .post(url)
            .bearer_auth(access_token)
            .json(input)
            .send()
            .await?
            .error_for_status()?)
    }
}

impl ESIClient {
    pub fn new(database: Arc<crate::DB>, client_id: String, client_secret: String) -> ESIClient {
        ESIClient {
            db: database,
            raw: ESIRawClient::new(client_id, client_secret),
        }
    }

    pub async fn process_auth(&self, grant_type: &str, token: &str) -> Result<i64, ESIError> {
        let result = self.raw.process_auth(grant_type, token).await?;
        self.save_auth(&result).await?;
        Ok(result.character_id)
    }

    async fn save_auth(&self, auth: &super::esi::AuthResult) -> Result<(), sqlx::Error> {
        let mut tx = self.db.begin().await?;

        if sqlx::query!("SELECT id FROM `character` WHERE id=?", auth.character_id)
            .fetch_optional(&mut tx)
            .await?
            .is_none()
        {
            sqlx::query!(
                "INSERT INTO `character` (id, name) VALUES (?, ?)",
                auth.character_id,
                auth.character_name
            )
            .execute(&mut tx)
            .await?;
        }

        let expiry_timestamp = auth.access_token_expiry.timestamp();
        let scopes_str = auth.scopes.join(" ");
        sqlx::query!(
            "REPLACE INTO access_token (character_id, access_token, expires, scopes) VALUES (?, ?, ?, ?)",
            auth.character_id,
            auth.access_token,
            expiry_timestamp,
            scopes_str,
        )
        .execute(&mut tx)
        .await?;

        sqlx::query!(
            "REPLACE INTO refresh_token (character_id, refresh_token) VALUES (?, ?)",
            auth.character_id,
            auth.refresh_token
        )
        .execute(&mut tx)
        .await?;

        tx.commit().await?;

        Ok(())
    }

    async fn access_token_raw(&self, character_id: i64) -> Result<(String, String), ESIError> {
        if let Some(record) = sqlx::query!(
            "SELECT * FROM access_token WHERE character_id=?",
            character_id
        )
        .fetch_optional(self.db.as_ref())
        .await?
        {
            if record.expires >= chrono::Utc::now().timestamp() {
                return Ok((record.access_token, record.scopes));
            }
        }

        let refresh = match sqlx::query!(
            "SELECT * FROM refresh_token WHERE character_id=?",
            character_id
        )
        .fetch_optional(self.db.as_ref())
        .await?
        {
            Some(r) => r,
            None => return Err(ESIError::NoToken),
        };
        let refreshed = match self
            .raw
            .process_auth("refresh_token", &refresh.refresh_token)
            .await
        {
            Ok(r) => r,
            Err(ESIError::HTTPError(e)) => {
                if e.is_status() && e.status().unwrap() == 400 {
                    warn!(
                        "Deleting refresh token for character {} as it failed to be used: {:?}",
                        character_id, e
                    );
                    let mut tx = self.db.begin().await?;
                    sqlx::query!(
                        "DELETE FROM access_token WHERE character_id=?",
                        character_id
                    )
                    .execute(&mut tx)
                    .await?;
                    sqlx::query!(
                        "DELETE FROM refresh_token WHERE character_id=?",
                        character_id
                    )
                    .execute(&mut tx)
                    .await?;
                    tx.commit().await?;

                    return Err(ESIError::NoToken);
                }
                return Err(e.into());
            }
            Err(e) => return Err(e),
        };
        self.save_auth(&refreshed).await?;

        Ok((refreshed.access_token, refreshed.scopes.join(" ")))
    }

    async fn access_token(&self, character_id: i64, scope: ESIScope) -> Result<String, ESIError> {
        let (token, scopes) = self.access_token_raw(character_id).await?;

        if !scopes.contains(scope.as_str()) {
            return Err(ESIError::MissingScope);
        }

        Ok(token)
    }

    pub async fn get<D: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        character_id: i64,
        scope: ESIScope,
    ) -> Result<D, ESIError> {
        let access_token = self.access_token(character_id, scope).await?;
        let url = format!("https://esi.evetech.net{}", path);
        Ok(self.raw.get(&url, &access_token).await?.json().await?)
    }

    pub async fn delete(
        &self,
        path: &str,
        character_id: i64,
        scope: ESIScope,
    ) -> Result<(), ESIError> {
        let access_token = self.access_token(character_id, scope).await?;
        let url = format!("https://esi.evetech.net{}", path);
        self.raw.delete(&url, &access_token).await?;
        Ok(())
    }

    pub async fn post<E: Serialize + ?Sized>(
        &self,
        path: &str,
        input: &E,
        character_id: i64,
        scope: ESIScope,
    ) -> Result<(), ESIError> {
        let access_token = self.access_token(character_id, scope).await?;
        let url = format!("https://esi.evetech.net{}", path);
        self.raw.post::<E>(&url, input, &access_token).await?;
        Ok(())
    }
}

pub mod fleet_members {
    use eve_data_core::TypeID;

    use crate::core::esi::ESIScope;

    use super::{ESIClient, ESIError};
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub struct ESIFleetMember {
        pub character_id: i64,
        pub ship_type_id: TypeID,
        pub squad_id: i64,
    }

    pub async fn get(
        client: &ESIClient,
        fleet_id: i64,
        boss_id: i64,
    ) -> Result<Vec<ESIFleetMember>, ESIError> {
        Ok(client
            .get(
                &format!("/v1/fleets/{}/members", fleet_id),
                boss_id,
                ESIScope::Fleets_ReadFleet_v1,
            )
            .await?)
    }
}
