//! Automatic replies (out of office), kept on the mail server as a Sieve
//! script through `ManageSieve` (port 4190, for example Dovecot on the
//! same host as IMAP), so they go out while every device is off.

use mail_core::vacation::{self, SCRIPT_NAME, Vacation};
use mail_store::{Account, AuthKind};
use mail_sync::sieve::SieveClient;
use mail_sync::{Credentials, SyncError, google, graph};
use serde::Serialize;

use crate::Ctx;
use crate::connections::{self, Service};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// An account's automatic replies.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutOfOffice {
    account_id: String,
    /// The server can send automatic replies for this account.
    available: bool,
    /// Why not, in plain words.
    message: Option<String>,
    /// Replies are on at the server now.
    active: bool,
    settings: Vacation,
}

fn unavailable(account: &Account, why: &str) -> OutOfOffice {
    OutOfOffice {
        account_id: account.id.clone(),
        available: false,
        message: Some(why.to_owned()),
        active: false,
        settings: Vacation::default(),
    }
}

/// Why an account has no filter connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoSieve {
    /// Gmail or Microsoft sign-in.
    NotPassword,
    /// No password stored to sign in with.
    NoPassword,
    /// The server doesn't offer `ManageSieve`.
    NotOffered,
}

/// A logged-in filter connection, or why there can't be one. Errors are
/// for problems worth trying again (a refused password, a lost network).
pub(crate) async fn open_sieve(
    state: &AppState,
    account: &Account,
) -> AppResult<Result<SieveClient, NoSieve>> {
    if account.auth != AuthKind::Password {
        return Ok(Err(NoSieve::NotPassword));
    }
    let Credentials::Password { username, password } =
        crate::credentials::for_account(state, account).await?
    else {
        return Ok(Err(NoSieve::NoPassword));
    };
    match SieveClient::connect(&account.imap.host, &username, &password).await {
        Ok(client) => Ok(Ok(client)),
        Err(SyncError::Connect { .. } | SyncError::Config(_) | SyncError::Protocol(_)) => {
            Ok(Err(NoSieve::NotOffered))
        }
        Err(other) => Err(other.into()),
    }
}

/// A logged-in filter connection that can send automatic replies, or why
/// there can't be one.
async fn connect(state: &AppState, account: &Account) -> AppResult<Result<SieveClient, String>> {
    match open_sieve(state, account).await? {
        Ok(client) if client.extensions.iter().any(|e| e == "vacation") => Ok(Ok(client)),
        Ok(client) => {
            client.logout().await;
            Ok(Err(
                "The mail server can't send automatic replies.".to_owned()
            ))
        }
        Err(NoSieve::NotPassword) => Ok(Err(
            "Connect this account's calendar and contacts to set its automatic replies here."
                .to_owned(),
        )),
        Err(NoSieve::NoPassword) => Ok(Err(
            "This account has no password to sign in with.".to_owned()
        )),
        Err(NoSieve::NotOffered) => Ok(Err(
            "The mail server doesn't offer automatic replies.".to_owned()
        )),
    }
}

async fn status(account: &Account, client: &mut SieveClient) -> AppResult<OutOfOffice> {
    let scripts = client.list().await?;
    let ours = scripts.iter().find(|s| s.name == SCRIPT_NAME);
    let settings = match ours {
        Some(_) => vacation::read(&client.get(SCRIPT_NAME).await?)
            .map(|(settings, _)| settings)
            .unwrap_or_default(),
        None => Vacation::default(),
    };
    Ok(OutOfOffice {
        account_id: account.id.clone(),
        available: true,
        message: None,
        active: ours.is_some_and(|s| s.active) && settings.enabled,
        settings,
    })
}

/// Whether replies set like this are going out today.
fn active_today(v: &Vacation) -> bool {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    v.enabled
        && v.start.as_deref().is_none_or(|s| s <= today.as_str())
        && v.end.as_deref().is_none_or(|e| e >= today.as_str())
}

fn provider_status(account: &Account, settings: Vacation) -> OutOfOffice {
    OutOfOffice {
        account_id: account.id.clone(),
        available: true,
        message: None,
        active: active_today(&settings),
        settings,
    }
}

/// Gmail and Microsoft accounts' replies, through Google's or Microsoft's
/// own settings once connected. `None` for other accounts.
async fn from_provider(
    state: &AppState,
    account: &Account,
    change: Option<&Vacation>,
) -> AppResult<Option<Result<OutOfOffice, String>>> {
    let offset = connections::local_offset();
    match connections::service(state, account)? {
        Service::Dav => Ok(None),
        Service::NotConnected => Ok(Some(Err(
            "Connect this account's calendar and contacts to set its automatic replies here."
                .to_owned(),
        ))),
        Service::Google => {
            if !connections::granted(state, account, "gmail.settings.basic") {
                return Ok(Some(Err(
                    "Connect this account's calendar and contacts again to set its automatic replies here.".to_owned(),
                )));
            }
            let client = connections::google_client(state, account).await?;
            let settings = match change {
                Some(v) => google::set_vacation(&client, v, offset).await?,
                None => google::get_vacation(&client, offset).await?,
            };
            Ok(Some(Ok(provider_status(account, settings))))
        }
        Service::Microsoft => {
            let client = connections::graph_client(state, account).await?;
            let now = i64::try_from(crate::state::now()).unwrap_or(0);
            let settings = match change {
                Some(v) => graph::set_auto_replies(&client, v, offset, now).await?,
                None => graph::get_auto_replies(&client, offset).await?,
            };
            Ok(Some(Ok(provider_status(account, settings))))
        }
    }
}

/// `get_out_of_office`: an account's automatic replies.
pub async fn get_out_of_office(ctx: &Ctx, account_id: String) -> AppResult<OutOfOffice> {
    let state = &ctx.state;
    let account = state.store.get_account(&account_id)?;
    match from_provider(state, &account, None).await? {
        Some(Ok(status)) => return Ok(status),
        Some(Err(why)) => return Ok(unavailable(&account, &why)),
        None => {}
    }
    let mut client = match connect(state, &account).await? {
        Ok(client) => client,
        Err(why) => return Ok(unavailable(&account, &why)),
    };
    let result = status(&account, &mut client).await;
    client.logout().await;
    result
}

/// `set_out_of_office`: save and switch automatic replies on or off.
pub async fn set_out_of_office(
    ctx: &Ctx,
    account_id: String,
    settings: Vacation,
) -> AppResult<OutOfOffice> {
    let state = &ctx.state;
    let settings = vacation::checked(settings).map_err(AppError::invalid)?;
    let account = state.store.get_account(&account_id)?;
    match from_provider(state, &account, Some(&settings)).await? {
        Some(Ok(status)) => return Ok(status),
        Some(Err(why)) => return Err(AppError::invalid(why)),
        None => {}
    }
    let mut client = match connect(state, &account).await? {
        Ok(client) => client,
        Err(why) => return Err(AppError::invalid(why)),
    };
    let result = save(&account, &mut client, &settings).await;
    client.logout().await;
    result
}

async fn save(
    account: &Account,
    client: &mut SieveClient,
    settings: &Vacation,
) -> AppResult<OutOfOffice> {
    let scripts = client.list().await?;
    let ours = scripts.iter().find(|s| s.name == SCRIPT_NAME).cloned();
    let running = scripts.iter().find(|s| s.active).map(|s| s.name.clone());
    // The filters that ran before ours keep running inside it.
    let previous = match (&ours, running) {
        (Some(o), _) if o.active => {
            vacation::read(&client.get(SCRIPT_NAME).await?).and_then(|(_, p)| p)
        }
        (_, running) => running.filter(|r| r != SCRIPT_NAME),
    };
    if previous.is_some() && !client.extensions.iter().any(|e| e == "include") {
        return Err(AppError::invalid(
            "Other mail filters are on for this account, and the server can't run them alongside automatic replies.",
        ));
    }
    let text = vacation::script(
        settings,
        std::slice::from_ref(&account.email),
        previous.as_deref(),
    );
    client.put(SCRIPT_NAME, &text).await?;
    let ours_running = ours.is_some_and(|o| o.active);
    if settings.enabled {
        client.set_active(SCRIPT_NAME).await?;
    } else if ours_running {
        client.set_active(previous.as_deref().unwrap_or("")).await?;
    }
    status(account, client).await
}
