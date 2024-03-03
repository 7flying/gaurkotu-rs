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
    #[command(description = "checks if there are any anime updates.")]
    CheckAnime,
    #[command(description = "updates the viewing progress of a series.")]
    UpdateAnime,
    #[command(description = "shows the animes that we are following.")]
    ShowFollowingAnime,
    #[command(description = "shows the animes that we have finished.")]
    ShowFinishedAnime,
    #[command(description = "gives a to-watch list to catch up on.")]
    ToWatch,
    #[command(description = "marks a given anime as finished.")]
    FinishAnime,
    #[command(description = "generates an id for a given name.")]
    GenId(String),
}

#[derive(Clone, Default)]
enum AnimeState {
    #[default]
    UpdateAnime,
    FinishAnime,
}

static FOLLOWING_FILE: &str = "anime-following.json";
static FINISHED_FILE: &str = "anime-finished.json";
static UPDATES_FILE: &str = "anime-updates.json";

#[tokio::main]
async fn main() {
    log::info!("Starting command bot...");
    let bot = Bot::from_env();
    Dispatcher::builder(bot, schema())
        .dependencies(dptree::deps![InMemStorage::<AnimeState>::new()])
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
        .branch(case![Command::ShowFinishedAnime].endpoint(command_show_finished_anime))
        .branch(case![Command::ToWatch].endpoint(command_to_watch))
        .branch(case![Command::FinishAnime].endpoint(command_finish_anime))
        .branch(case![Command::GenId(anime)].endpoint(command_gen_id));
    let message_handler = Update::filter_message()
        .branch(command_handler)
        .branch(dptree::endpoint(invalid_state));
    let callback_query_handler = Update::filter_callback_query()
        .branch(case![AnimeState::UpdateAnime].endpoint(update_given_anime))
        .branch(case![AnimeState::FinishAnime].endpoint(finish_given_anime));

    dialogue::enter::<Update, InMemStorage<AnimeState>, AnimeState, _>()
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

async fn gen_following_keyboard() -> InlineKeyboardMarkup {
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
    InlineKeyboardMarkup::new(buttons)
}

/// handles /updateanime
async fn command_update_anime(
    bot: Bot,
    dialogue: Dialogue<AnimeState, InMemStorage<AnimeState>>,
    msg: Message,
) -> HandlerResult {
    if !is_allowed_user(msg.chat.id) {
        return Ok(());
    }
    let animes = gen_following_keyboard().await;
    bot.send_message(msg.chat.id, "Which anime do you want to update?")
        .reply_markup(animes)
        .await?;
    dialogue.update(AnimeState::UpdateAnime).await?;
    Ok(())
}

// Works along with /updateanime to update the progress on a given anime series
async fn update_given_anime(
    bot: Bot,
    dialogue: Dialogue<AnimeState, InMemStorage<AnimeState>>,
    q: CallbackQuery,
) -> HandlerResult {
    if let Some(anime) = &q.data {
        let store_dir = env::var("BOT_STORAGE").expect("Error checking BOT_STORAGE");
        let json_follows = store_dir.to_owned() + "/" + FOLLOWING_FILE;
        let follows_content = tokio::fs::read(json_follows)
            .await
            .expect("Error reading following file");
        let mut following: Follows =
            serde_json::from_slice(&follows_content).expect("Error deserializing following json");
        if !following.following.contains_key(anime) {
            bot.send_message(
                dialogue.chat_id(),
                format!("I couldn't find {anime} in our follows"),
            )
            .await?;
            dialogue.exit().await?;
            return Ok(());
        }

        let mut info = following.following.get(anime).unwrap().to_owned();
        info.info.last_episode += 1;
        following
            .following
            .insert(anime.to_owned(), info.to_owned());

        let mut file = File::create(store_dir.to_owned() + "/" + FOLLOWING_FILE).await?;
        file.write_all(serde_json::to_string_pretty(&following)?.as_bytes())
            .await?;
        bot.send_message(
            dialogue.chat_id(),
            format!(
                "Updated '{}' to episode {}",
                info.extra.en_name, info.info.last_episode
            ),
        )
        .await?;
        dialogue.exit().await?;
    }
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
    if stuff.is_empty() {
        bot.send_message(msg.chat.id, "We are not following any anime series.")
            .await?;
        return Ok(());
    }
    stuff.sort_unstable();
    let mut ret = "We are following these anime series:\n\n".to_string();
    for aniinfo in stuff {
        ret.push_str(&format!(
            "— {} [{}] - Ep. {}\n",
            aniinfo.extra.en_name, aniinfo.extra.season, aniinfo.info.last_episode
        ));
    }
    bot.send_message(msg.chat.id, ret).await?;
    Ok(())
}

/// handles /showfinishedanime
async fn command_show_finished_anime(bot: Bot, msg: Message) -> Result<()> {
    if !is_allowed_user(msg.chat.id) {
        return Ok(());
    }
    let follows_data = read_from_storage("anime-finished.json").await;
    let following: Follows =
        serde_json::from_slice(&follows_data).expect("Error deserializing finished json");
    let mut stuff: Vec<AniInfo> = following.following.values().cloned().collect();
    if stuff.is_empty() {
        bot.send_message(msg.chat.id, "We haven't finished any anime series.")
            .await?;
        return Ok(());
    }
    stuff.sort_unstable();
    let mut ret = "We have finished these anime series:\n\n".to_string();
    for aniinfo in stuff {
        ret.push_str(&format!(
            "— {} [{}] - Ep. {}\n",
            aniinfo.extra.en_name, aniinfo.extra.season, aniinfo.info.last_episode
        ));
    }
    bot.send_message(msg.chat.id, ret).await?;
    Ok(())
}

/// handles /finishanime
async fn command_finish_anime(
    bot: Bot,
    dialogue: Dialogue<AnimeState, InMemStorage<AnimeState>>,
    msg: Message,
) -> HandlerResult {
    if !is_allowed_user(msg.chat.id) {
        return Ok(());
    }
    let animes = gen_following_keyboard().await;
    bot.send_message(msg.chat.id, "Which anime have you finished?")
        .reply_markup(animes)
        .await?;
    dialogue.update(AnimeState::FinishAnime).await?;
    Ok(())
}

// works along with /finishanime to mark a given anime as finished
async fn finish_given_anime(
    bot: Bot,
    dialogue: Dialogue<AnimeState, InMemStorage<AnimeState>>,
    q: CallbackQuery,
) -> HandlerResult {
    if let Some(anime) = &q.data {
        let store_dir = env::var("BOT_STORAGE").expect("Error checking BOT_STORAGE");
        // get following
        let json_follows = store_dir.to_owned() + "/" + FOLLOWING_FILE;
        let follows_content = tokio::fs::read(json_follows)
            .await
            .expect("Error reading following file");
        let mut following: Follows =
            serde_json::from_slice(&follows_content).expect("Error deserializing following json");
        // get finished
        let json_finished = store_dir.to_owned() + "/" + FINISHED_FILE;
        let finished_content = tokio::fs::read(json_finished)
            .await
            .expect("Error reading finished file");
        let mut finished: Follows =
            serde_json::from_slice(&finished_content).expect("Error deserializing finished json");
        if finished.following.contains_key(anime) {
            bot.send_message(
                dialogue.chat_id(),
                format!("You already have '{anime}' in our finished list"),
            )
            .await?;
            dialogue.exit().await?;
            return Ok(());
        }
        // add it to finished
        let info = following.following.get(anime).unwrap().clone();
        finished.following.insert(anime.to_owned(), info.to_owned());
        let mut file_finished = File::create(store_dir.to_owned() + "/" + FINISHED_FILE).await?;
        file_finished
            .write_all(serde_json::to_string_pretty(&finished)?.as_bytes())
            .await?;
        // remove from following and update
        following.following.remove(anime);
        let mut file_following = File::create(store_dir.to_owned() + "/" + FOLLOWING_FILE).await?;
        file_following
            .write_all(serde_json::to_string_pretty(&following)?.as_bytes())
            .await?;
        bot.send_message(
            dialogue.chat_id(),
            format!(
                "'{}' has been added to the finished list.",
                info.extra.en_name
            ),
        )
        .await?;
    } else {
        bot.send_message(dialogue.chat_id(), "Did not get an anime")
            .await?;
    }
    dialogue.exit().await?;
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
                        "just Ep. {}",
                        updates.updates.get(&id).unwrap().last_episode
                    ),
                ));
            } else {
                towatch.push((
                    ani.extra.en_name,
                    format!(
                        "from {} up to Ep.{}",
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
            ret.push_str(&format!("· {ani}\n    → {desc}\n"));
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
    let mut store_update: HashMap<&String, &AniMinInfo> = HashMap::new();
    let mut message_update: HashMap<&String, &AniMinInfo> = HashMap::new();
    let mut new_series: Vec<&String> = Vec::new();
    for (id, ani) in eps.iter() {
        if !following.following.contains_key(id) {
            if ani.last_episode == 1 {
                new_series.push(&ani.name);
            }
            continue;
        }
        if (!updates.updates.contains_key(id)
            && ani.last_episode > following.following.get(id).unwrap().info.last_episode)
            || (updates.updates.contains_key(id)
                && ani.last_episode > updates.updates.get(id).unwrap().last_episode)
        {
            store_update.insert(id, ani);
            message_update.insert(&following.following.get(id).unwrap().extra.en_name, ani);
        }
    }
    if message_update.values().len() == 0 && new_series.is_empty() {
        bot.send_message(chat_id, "There are no updates!")
            .await
            .unwrap();
    } else {
        let mut message: String = "This is the latest anime update:\n\n".to_owned();
        let mut up = false;
        for (ename, info) in message_update {
            message.push_str(&format!(
                "— Ep. {} for '{}' is out ({})\n",
                info.last_episode, ename, info.name
            ));
            up = true;
        }
        if !new_series.is_empty() {
            if up {
                message.push('\n');
            }
            message.push_str("We have new series coming up!\n");
            for series in new_series {
                message.push_str(&format!("— {series}\n"));
            }
        }
        bot.send_message(chat_id, message).await?;
        sync_updates(updates, store_update).await?;
    }
    Ok(())
}

async fn sync_updates(mut updates: Updates, notify: HashMap<&String, &AniMinInfo>) -> Result<()> {
    for (id, info) in notify {
        updates.updates.insert(id.to_owned(), info.to_owned());
    }
    let store_dir = env::var("BOT_STORAGE").expect("Error checking BOT_STORAGE");
    let json = serde_json::to_string_pretty(&updates)?;
    let mut file = File::create(store_dir.to_owned() + "/" + UPDATES_FILE).await?;
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
    let mut updates: HashMap<String, AniMinInfo> = HashMap::new();
    let maybe_feed = parser::parse(stuff.as_bytes());
    if maybe_feed.is_err() {
        return Ok(updates);
    }
    let feed = maybe_feed.unwrap();
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
