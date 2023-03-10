use feed_rs::parser;
use hyper::{body::HttpBody, Client};
use hyper_tls::HttpsConnector;
use regex::Regex;
use slug::slugify;
use std::collections::HashMap;
use std::env;
use std::str;
use teloxide::dispatching::dialogue::InMemStorage;
use teloxide::dispatching::{dialogue, UpdateHandler};
use teloxide::types::InlineKeyboardButton;
use teloxide::types::InlineKeyboardMarkup;
use teloxide::{prelude::*, utils::command::BotCommands};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

mod anime;
use anime::{AniInfo, AniMinInfo, Follows, Updates, ANIME_RSS};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type HandlerResult = std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
enum Command {
    #[command(description = "shows this text.")]
    Help,
    #[command(description = "check if there are any anime updates.")]
    CheckAnime,
    #[command(description = "updates the viewing progress of a series.")]
    UpdateAnime,
    #[command(description = "shows the animes that we are following.")]
    ShowFollowingAnime,
    #[command(description = "gives a to-watch list to catch up on.")]
    ToWatch,
    #[command(description = "generate an id for a given name.")]
    GenId(String),
}

#[derive(Clone, Default)]
enum UpdateAnimeState {
    #[default]
    UpdateAnime,
}

#[tokio::main]
async fn main() {
    log::info!("Starting command bot...");
    let bot = Bot::from_env();
    Dispatcher::builder(bot, schema())
        .dependencies(dptree::deps![InMemStorage::<UpdateAnimeState>::new()])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

fn schema() -> UpdateHandler<Box<dyn std::error::Error + Send + Sync + 'static>> {
    use dptree::case;
    let command_handler = teloxide::filter_command::<Command, _>()
        .branch(case![Command::Help].endpoint(command_help))
        .branch(case![Command::CheckAnime].endpoint(command_check_anime))
        .branch(case![Command::UpdateAnime].endpoint(command_update_anime))
        .branch(case![Command::ShowFollowingAnime].endpoint(command_show_following_anime))
        .branch(case![Command::ToWatch].endpoint(command_to_watch))
        .branch(case![Command::GenId(anime)].endpoint(command_gen_id));
    let message_handler = Update::filter_message()
        .branch(command_handler)
        .branch(dptree::endpoint(invalid_state));
    let callback_query_handler = Update::filter_callback_query()
        .branch(case![UpdateAnimeState::UpdateAnime].endpoint(update_given_anime));

    dialogue::enter::<Update, InMemStorage<UpdateAnimeState>, UpdateAnimeState, _>()
        .branch(message_handler)
        .branch(callback_query_handler)
}

fn is_allowed_user(msg_id: ChatId) -> bool {
    let chat_id = env::var("TCHAT_ID");
    match chat_id {
        Ok(id) => {
            if msg_id != ChatId(id.parse::<i64>().unwrap()) {
                return false;
            }
        }
        _ => return false,
    }
    true
}

/// handles /help
async fn command_help(bot: Bot, msg: Message) -> Result<()> {
    if !is_allowed_user(msg.chat.id) {
        return Ok(());
    }
    bot.send_message(msg.chat.id, Command::descriptions().to_string())
        .await?;
    Ok(())
}

/// handles /checkanime
async fn command_check_anime(bot: Bot, msg: Message) -> Result<()> {
    if !is_allowed_user(msg.chat.id) {
        return Ok(());
    }
    check_updates(msg.chat.id, &bot).await
}

/// handles /updateanime
async fn command_update_anime(
    bot: Bot,
    dialogue: Dialogue<UpdateAnimeState, InMemStorage<UpdateAnimeState>>,
    msg: Message,
) -> HandlerResult {
    if !is_allowed_user(msg.chat.id) {
        return Ok(());
    }
    let mut follows = get_follows_vec().await;
    follows.sort_by_key(|k| k.1.to_owned());
    let mut buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for f in follows {
        let name = if f.1.len() <= 128 {
            f.1
        } else {
            f.1[..128].to_owned()
        };
        buttons.push([InlineKeyboardButton::callback(name, f.0.to_string())].to_vec());
    }
    let animes = InlineKeyboardMarkup::new(buttons);
    bot.send_message(msg.chat.id, "Which anime do you want to update?")
        .reply_markup(animes)
        .await?;
    dialogue.update(UpdateAnimeState::UpdateAnime).await?;
    Ok(())
}

/// handles /showfollowinganime
async fn command_show_following_anime(bot: Bot, msg: Message) -> Result<()> {
    if !is_allowed_user(msg.chat.id) {
        return Ok(());
    }
    let follows_data = read_from_storage("anime-following.json").await;
    let following: Follows =
        serde_json::from_slice(&follows_data).expect("Error deserializing follows json");
    let mut stuff: Vec<AniInfo> = following.following.values().cloned().collect();
    stuff.sort_unstable();
    let mut ret = String::new();
    for aniinfo in stuff {
        ret.push_str(&format!(
            "??? {}, {} - Episode {}\n",
            aniinfo.extra.en_name, aniinfo.extra.season, aniinfo.info.last_episode
        ));
    }
    bot.send_message(msg.chat.id, ret).await?;
    Ok(())
}

/// handles /towatch
async fn command_to_watch(bot: Bot, msg: Message) -> Result<()> {
    if !is_allowed_user(msg.chat.id) {
        return Ok(());
    }
    let follows_data = read_from_storage("anime-following.json").await;
    let following: Follows =
        serde_json::from_slice(&follows_data).expect("Error deserializing following json");
    let updates_data = read_from_storage("anime-updates.json").await;
    let updates: Updates =
        serde_json::from_slice(&updates_data).expect("Error deserializing updates json");
    let mut towatch: Vec<(String, String)> = Vec::new();
    for (id, ani) in following.following {
        if updates.updates.contains_key(&id)
            && ani.info.last_episode < updates.updates.get(&id).unwrap().last_episode
        {
            if ani.info.last_episode + 1 == updates.updates.get(&id).unwrap().last_episode {
                towatch.push((
                    ani.extra.en_name,
                    format!(
                        "just Episode {}",
                        updates.updates.get(&id).unwrap().last_episode
                    ),
                ));
            } else {
                towatch.push((
                    ani.extra.en_name,
                    format!(
                        "from {} up to Episode {}",
                        ani.info.last_episode + 1,
                        updates.updates.get(&id).unwrap().last_episode
                    ),
                ));
            }
        }
    }
    if !towatch.is_empty() {
        towatch.sort_unstable();
        let mut ret = "This is our watchlist:\n".to_string();
        for (ani, desc) in towatch {
            ret.push_str(&format!("?? {ani}\n    ??? {desc}\n"));
        }
        bot.send_message(msg.chat.id, ret).await?;
    } else {
        bot.send_message(
            msg.chat.id,
            "We are up to date according to the latest Update data.",
        )
        .await?;
    }

    Ok(())
}

/// handles /genid {anime}
async fn command_gen_id(bot: Bot, msg: Message, anime: String) -> Result<()> {
    if !is_allowed_user(msg.chat.id) {
        return Ok(());
    }
    bot.send_message(
        msg.chat.id,
        format!("id:{:x}", md5::compute(slugify(anime))),
    )
    .await?;
    Ok(())
}

async fn update_given_anime(
    bot: Bot,
    dialogue: Dialogue<UpdateAnimeState, InMemStorage<UpdateAnimeState>>,
    q: CallbackQuery,
) -> HandlerResult {
    if let Some(anime) = &q.data {
        let store_dir = env::var("BOT_STORAGE").expect("Error checking BOT_STORAGE");
        let json_follows = store_dir.to_owned() + "/anime-following.json";
        let follows_content = tokio::fs::read(json_follows)
            .await
            .expect("Error reading following file");
        let mut following: Follows =
            serde_json::from_slice(&follows_content).expect("Error deserializing following json");
        if following.following.contains_key(anime) {
            let mut info = following.following.get(anime).unwrap().to_owned();
            info.info.last_episode += 1;
            following
                .following
                .insert(anime.to_owned(), info.to_owned());

            let json = serde_json::to_string_pretty(&following)?;
            let mut file = File::create(store_dir.to_owned() + "/anime-following.json").await?;
            file.write_all(json.as_bytes()).await?;
            bot.send_message(
                dialogue.chat_id(),
                format!(
                    "Updated '{}' to episode {}",
                    info.extra.en_name, info.info.last_episode
                ),
            )
            .await?;
        } else {
            bot.send_message(
                dialogue.chat_id(),
                format!("I couldn't find {anime} in our follows"),
            )
            .await?;
        }

        dialogue.exit().await?;
    }
    Ok(())
}

async fn invalid_state(bot: Bot, msg: Message) -> HandlerResult {
    bot.send_message(
        msg.chat.id,
        "Unable to handle the message. Type /help to see the usage.",
    )
    .await?;
    Ok(())
}

async fn check_updates(chat_id: ChatId, bot: &Bot) -> Result<()> {
    let updates_content = read_from_storage("anime-updates.json").await;
    let updates: Updates =
        serde_json::from_slice(&updates_content).expect("Error deserializing update json");
    let follows_content = read_from_storage("anime-following.json").await;
    let following: Follows =
        serde_json::from_slice(&follows_content).expect("Error deserializing following json");

    let eps = fetch_rss().await?;
    // we care about the ones that we are following, and out of those, the new updates
    let mut store_update: HashMap<String, AniMinInfo> = HashMap::new();
    let mut message_update: HashMap<String, AniMinInfo> = HashMap::new();
    for (id, ani) in eps.iter() {
        if !following.following.contains_key(id) {
            continue;
        }
        if (!updates.updates.contains_key(id)
            && ani.last_episode > following.following.get(id).unwrap().info.last_episode)
            || (updates.updates.contains_key(id)
                && ani.last_episode > updates.updates.get(id).unwrap().last_episode)
        {
            store_update.insert(id.to_owned(), ani.to_owned());
            message_update.insert(
                following
                    .following
                    .get(id)
                    .unwrap()
                    .extra
                    .en_name
                    .to_owned(),
                ani.to_owned(),
            );
        }
    }
    if message_update.values().len() == 0 {
        bot.send_message(chat_id, "There are no updates!")
            .await
            .unwrap();
    } else {
        let mut message: String = "This is the latest anime update:\n".to_owned();
        for (ename, info) in message_update {
            message.push_str(&format!(
                "??? Episode {} for '{}' is out ({})\n",
                info.last_episode, ename, info.name
            ));
        }
        bot.send_message(chat_id, message).await.unwrap();
        sync_updates(updates, store_update).await?;
    }

    Ok(())
}

async fn sync_updates(mut updates: Updates, notify: HashMap<String, AniMinInfo>) -> Result<()> {
    for (id, info) in notify {
        updates.updates.insert(id, info);
    }
    let store_dir = env::var("BOT_STORAGE").expect("Error checking BOT_STORAGE");
    let json = serde_json::to_string_pretty(&updates)?;
    let mut file = File::create(store_dir.to_owned() + "/anime-updates.json").await?;
    file.write_all(json.as_bytes()).await?;
    Ok(())
}

async fn fetch_rss() -> Result<HashMap<String, AniMinInfo>> {
    let https = HttpsConnector::new();
    let client = Client::builder().build::<_, hyper::Body>(https);
    let uri = ANIME_RSS.parse()?;
    let mut resp = client.get(uri).await.expect("Error fetching RSS");
    let mut stuff = String::new();
    while let Some(next) = resp.data().await {
        let chunk = next?;
        stuff.push_str(str::from_utf8(&chunk)?);
    }
    let feed = parser::parse(stuff.as_bytes()).unwrap();
    let mut updates: HashMap<String, AniMinInfo> = HashMap::new();
    let re = Regex::new(r"([\w\W\s]+) - Episode ([\d\D]+)").unwrap();
    for et in feed.entries {
        if let Some(info) = re.captures(&et.title.unwrap().content) {
            let episode = info.get(2).map_or("", |m| m.as_str());
            let series = info.get(1).map_or("", |m| m.as_str());
            updates.insert(
                format!("{:x}", md5::compute(slugify(series))),
                AniMinInfo {
                    name: String::from(series),
                    last_episode: episode.parse::<i16>().unwrap(),
                },
            );
        }
    }
    Ok(updates)
}

async fn get_follows_vec() -> Vec<(String, String)> {
    let follows_content = read_from_storage("anime-following.json").await;
    let following: Follows =
        serde_json::from_slice(&follows_content).expect("Error deserializing following json");
    let mut ret: Vec<(String, String)> = vec![];
    for (key, val) in following.following {
        ret.push((key, val.extra.en_name));
    }
    ret
}

async fn read_from_storage(file_name: &str) -> Vec<u8> {
    let store_dir = env::var("BOT_STORAGE").expect("Error checking BOT_STORAGE");
    let path = store_dir.to_owned() + "/" + file_name;
    tokio::fs::read(path)
        .await
        .expect("Error reading {file_name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn xml_required_fields() -> Result<()> {
        let https = HttpsConnector::new();
        let client = Client::builder().build::<_, hyper::Body>(https);
        let uri = ANIME_RSS.parse().unwrap();
        let mut resp = client.get(uri).await.expect("Error fetching RSS");
        let mut stuff = String::new();
        while let Some(next) = resp.data().await {
            let chunk = next.unwrap();
            stuff.push_str(str::from_utf8(&chunk).unwrap());
        }
        let feed = parser::parse(stuff.as_bytes()).unwrap();
        let re = Regex::new(r"([\w\W\s]+) - Episode ([\d\D]+)").unwrap();
        if feed.entries.len() == 0 {
            assert!(false);
        }
        for et in feed.entries {
            match re.captures(&et.title.unwrap().content) {
                Some(info) => {
                    let episode = info.get(2).map_or("", |m| m.as_str());
                    let series = info.get(1).map_or("", |m| m.as_str());
                    if episode.len() == 0 || series.len() == 0 {
                        assert!(false);
                    }
                }
                None => assert!(false),
            }
            break;
        }
        Ok(())
    }

    #[test]
    fn serde_serialization() -> Result<()> {
        let json_following = r#"{
  "following": {
    "098f6bcd4621d373cade4e832627b4f6": {
      "info": {
        "name": "Some name",
        "last_episode": 1
      },
      "extra": {
        "en_name": "Some en name",
        "season": {
          "Autumn": 2022
        }
      }
    }
  }
}"#;
        let json_updates = r#"{
  "updates": {
    "098f6bcd4621d373cade4e832627b4f6": {
      "name": "Some en name",
      "last_episode": 7
    }
  }
}"#;
        // deserialization
        let following: Follows = serde_json::from_str(json_following).unwrap();
        let updates: Updates = serde_json::from_str(json_updates).unwrap();
        // serialization
        let gen_following = serde_json::to_string_pretty(&following).unwrap();
        let gen_updates = serde_json::to_string_pretty(&updates).unwrap();
        assert_eq!(gen_following, json_following);
        assert_eq!(gen_updates, json_updates);
        Ok(())
    }
}
