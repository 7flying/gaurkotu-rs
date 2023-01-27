use feed_rs::parser;
use hyper::{body::HttpBody, Client};
use hyper_tls::HttpsConnector;
use regex::Regex;
use serde::{Deserialize, Serialize};
use slug::slugify;
use std::collections::HashMap;
use std::env;
use std::str;
use teloxide::{prelude::*, utils::command::BotCommands};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const ANIME_RSS: &str = "https://raw.githubusercontent.com/ArjixGamer/gogoanime-rss/main/gogoanime/gogoanime-rss-sub.xml";

#[derive(Debug, Serialize, Deserialize)]
struct Follows {
    //#[serde(borrow = "'a")]
    following: HashMap<String, AniInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Updates {
    updates: HashMap<String, AniInfo>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AniInfo {
    info: AniMinInfo,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AniMinInfo {
    name: String,
    last_episode: i16,
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    log::info!("Starting command bot...");
    let bot = Bot::from_env();
    Command::repl(bot, handle_command).await;

    Ok(())
}

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
enum Command {
    #[command(description = "display this text.")]
    Help,
    #[command(description = "check if there are any anime updates.")]
    CheckAnime,
}

async fn handle_command(bot: Bot, msg: Message, cmd: Command) -> ResponseResult<()> {
    // ignore messages from everyone else but us
    let chat_id = env::var("TCHAT_ID");
    match chat_id {
        Ok(id) => {
            if msg.chat.id != ChatId(id.parse::<i64>().unwrap()) {
                return Ok(());
            }
        }
        _ => return Ok(()),
    }
    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        Command::CheckAnime => {
            command_check_anime(msg.chat.id, bot)
                .await
                .expect("Error checking anime updates");
        }
    };
    Ok(())
}

async fn command_check_anime(chat_id: ChatId, bot: Bot) -> Result<()> {
    check_updates(chat_id, &bot).await
}

async fn check_updates(chat_id: ChatId, bot: &Bot) -> Result<()> {
    let store_dir = env::var("BOT_STORAGE").expect("Error checking BOT_STORAGE");
    let json_updates = store_dir.to_owned() + "/anime-updates.json";
    let updates_content = tokio::fs::read(json_updates)
        .await
        .expect("Error reading updates file");
    let updates: Updates =
        serde_json::from_slice(&updates_content).expect("Error deserializing update json");
    let json_follows = store_dir.to_owned() + "/anime-following.json";
    let follows_content = tokio::fs::read(json_follows)
        .await
        .expect("Error reading following file");
    let following: Follows =
        serde_json::from_slice(&follows_content).expect("Error deserializing following json");

    let eps = fetch_rss().await?;
    // we care about the ones that we are following, and out of those, the new updates
    let mut notify: HashMap<String, AniInfo> = HashMap::new();
    for (id, ani) in eps.iter() {
        if !following.following.contains_key(id) {
            continue;
        }
        if (!updates.updates.contains_key(id)
            && ani.info.last_episode > following.following.get(id).unwrap().info.last_episode)
            || (updates.updates.contains_key(id)
                && ani.info.last_episode > updates.updates.get(id).unwrap().info.last_episode)
        {
            notify.insert(id.to_owned(), ani.to_owned());
        }
    }

    let mut message: String = "This is the latest anime update:\n".to_owned();
    for ep in notify.values() {
        message.push_str(&format!(
            "— Episode {} for {} is out\n",
            ep.info.last_episode, ep.info.name
        ));
    }
    bot.send_message(chat_id, message).await.unwrap();
    sync_updates(updates, notify, store_dir).await?;
    Ok(())
}

async fn sync_updates(
    mut updates: Updates,
    notify: HashMap<String, AniInfo>,
    store_dir: String,
) -> Result<()> {
    for (id, info) in notify {
        updates.updates.insert(id, info);
    }
    let json = serde_json::to_string_pretty(&updates)?;
    let mut file = File::create(store_dir.to_owned() + "/anime-updates.json").await?;
    file.write_all(json.as_bytes()).await?;
    Ok(())
}

async fn fetch_rss() -> Result<HashMap<String, AniInfo>> {
    let https = HttpsConnector::new();
    let client = Client::builder().build::<_, hyper::Body>(https);
    let uri = ANIME_RSS.parse()?;
    let mut resp = client.get(uri).await.expect("Error fetching RSS");
    let mut stuff = "".to_owned();
    // TODO: do not convert to str since we are using bytes below
    while let Some(next) = resp.data().await {
        let chunk = next?;
        stuff.push_str(str::from_utf8(&chunk)?);
    }
    let feed = parser::parse(stuff.as_bytes()).unwrap();
    let mut updates: HashMap<String, AniInfo> = HashMap::new();
    let re = Regex::new(r"([\w\W\s]+) - Episode ([\d\D]+)").unwrap();
    for et in feed.entries {
        if let Some(info) = re.captures(&et.title.unwrap().content) {
            let episode = info.get(2).map_or("", |m| m.as_str());
            let series = info.get(1).map_or("", |m| m.as_str());
            updates.insert(
                slugify(&series),
                AniInfo {
                    info: AniMinInfo {
                        name: String::from(series),
                        last_episode: episode.parse::<i16>().unwrap(),
                    },
                },
            );
        }
    }
    Ok(updates)
}
