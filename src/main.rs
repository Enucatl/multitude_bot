use std::env;
use std::error::Error;
use std::fs;

use rss::validation::Validate;
use rss::Channel;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, Database, DatabaseConnection, DbErr, DeleteResult,
    EntityTrait, QueryFilter, Set,
};
use teloxide::{
    dispatching::{HandlerExt, UpdateFilterExt},
    dptree,
    payloads::SendMessageSetters,
    prelude::{Bot, Dispatcher, LoggingErrorHandler, Requester, ResponseResult, Update},
    types::{ChatId, Message, ParseMode},
    utils::command::BotCommands,
};
use tokio_schedule::{every, Job};
use urlencoding::encode;

use entity::{chat, feed};
use migration::{Migrator, MigratorTrait};

const TELOXIDE_TOKEN_PATH: &str = "/run/secrets/teloxide_token";

async fn db_connect() -> Result<DatabaseConnection, DbErr> {
    let db_user = env::var("DB_USER").expect("DB_USER environment variable not set");
    let db_password_file =
        env::var("DB_PASSWORD_FILE").expect("DB_PASSWORD_FILE environment variable not set");
    let db_password = fs::read_to_string(&db_password_file)
        .expect(&format!("Couldn't read file {}", &db_password_file));
    // Encode the password to escape special characters
    let db_password = encode(&db_password);
    let db_host = env::var("DB_HOST").expect("DB_HOST environment variable not set");
    let db_name = env::var("DB_NAME").expect("DB_NAME environment variable not set");
    let db_url = format!(
        "postgres://{}:{}@{}:5432/{}",
        &db_user, &db_password, &db_host, &db_name
    );
    let db = Database::connect(&db_url).await?;
    Ok(db)
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    // Connect to database
    log::info!("Connecting to database...");
    let db = db_connect().await.expect("Can't connect to database");
    assert!(db.ping().await.is_ok());

    // Apply any new migrations to the database
    Migrator::up(&db, None).await.expect("Migrations failed");

    // Start the bot
    log::info!("Starting command bot...");
    let teloxide_token = fs::read_to_string(TELOXIDE_TOKEN_PATH)
        .expect(&format!("Couldn't read file {}", TELOXIDE_TOKEN_PATH));
    let bot = Bot::new(teloxide_token);

    // Check for feed updates
    let bot_clone = bot.clone();
    let db_clone = db.clone();
    let every_30_seconds = every(30)
        .seconds()
        .perform(move || check_for_updates(bot_clone.clone(), db_clone.clone()));
    tokio::spawn(every_30_seconds);

    let handler = dptree::entry()
        .branch(
            // Filter messages from users who are not in the DB "logged out"
            Update::filter_message()
                .filter_async(is_not_subscribed)
                .branch(
                    Update::filter_message()
                        .filter_command::<LoggedOutCommand>()
                        .endpoint(process_logged_out_command),
                )
                .branch(dptree::entry().endpoint(ask_to_subscribe)),
        )
        .branch(
            Update::filter_message()
                .filter_command::<LoggedInCommand>()
                .endpoint(process_command),
        )
        .branch(
            // Handle other messages or actions here
            dptree::filter(|msg: Message| msg.chat.is_group() || msg.chat.is_supergroup())
                .endpoint(noop),
        );

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![db])
        .default_handler(|upd| async move {
            log::warn!("Unhandled update: {:?}", upd);
        })
        .error_handler(LoggingErrorHandler::with_custom_text(
            "An error has occurred in the dispatcher",
        ))
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

/// Periodically checks for updates in RSS feeds and sends messages for new items.
///
/// This function takes a Telegram `Bot` instance and a database connection `DatabaseConnection`
/// to fetch and process RSS feeds for updates. It fetches each RSS feed, parses it,
/// checks for new items, and sends messages for new items to the corresponding Telegram chats.
/// If any errors occur during the process, they are logged to the console.
///
/// # Arguments
///
/// * `bot` - A `Bot` instance for sending messages.
/// * `db` - A `DatabaseConnection` for fetching feed and chat information.
///
/// # Example
///
/// ```rust
/// check_for_updates(bot, db).await;
/// ```
async fn check_for_updates(bot: Bot, db: DatabaseConnection) {
    println!("Every 30 seconds!");
    let feeds = entity::prelude::Feed::find().all(&db).await;
    if let Err(err) = feeds {
        println!("Error fetching feeds: {:?}", err);
        return;
    }

    for feed in feeds.unwrap() {
        let content = reqwest::get(&feed.link).await;
        if let Err(err) = content {
            println!("Error fetching content: {:?}", err);
            continue;
        }
        let content = content.unwrap();
        let content = content.bytes().await;
        if let Err(err) = content {
            println!("Error reading bytes: {:?}", err);
            continue;
        }
        let content = content.unwrap();
        let channel = Channel::read_from(&content[..]);
        if let Err(err) = channel {
            println!("Error parsing channel: {:?}", err);
            continue;
        }
        let channel = channel.unwrap();
        let mut max_update_time: Option<sea_orm::prelude::DateTime> = None;

        for item in channel.items {
            let published_date = item.pub_date().unwrap_or_default();
            let published_date = rfc822_sanitizer::parse_from_rfc2822_with_fallback(published_date)
                .unwrap_or_default();
            let published_date = published_date.naive_utc();
            if published_date > feed.updated_at {
                let mut message = String::new();
                let link = item.link.unwrap_or("".to_string());
                let title = item.title.unwrap_or("".to_string());
                message.push_str(&format!("<i>{}</i>\n", feed.title));
                message.push_str(&format!("<a href='{}'>{}</a>\n", link, title));
                if let Err(err) = bot
                    .send_message(ChatId(feed.chat_id), &message)
                    .parse_mode(ParseMode::Html)
                    .await
                {
                    println!("Error sending message: {:?}", err);
                }
                if max_update_time.is_none() || published_date > max_update_time.unwrap() {
                    max_update_time = Some(published_date);
                }
            }
        }
        if let Some(max_time) = max_update_time {
            if max_time > feed.updated_at {
                let mut updated_feed: feed::ActiveModel = feed.into();
                updated_feed.updated_at = Set(max_time);
                let updated_feed = updated_feed.update(&db).await;
                if let Err(err) = updated_feed {
                    println!("Error updating feed: {:?}", err);
                    continue;
                }
            }
        }
    }
}

async fn ask_to_subscribe(bot: Bot, msg: Message) -> ResponseResult<()> {
    bot.send_message(
        msg.chat.id,
        "type /start to create an account and chat with the bot. Only this chat id will be stored.",
    )
    .await?;
    Ok(())
}

async fn noop(_bot: Bot, _msg: Message) -> ResponseResult<()> {
    // no action on other messages
    Ok(())
}

async fn is_not_subscribed(msg: Message, db: DatabaseConnection) -> bool {
    // check if the chat is not in the database
    let c: Option<chat::Model> = entity::prelude::Chat::find_by_id(msg.chat.id.0)
        .one(&db)
        .await
        .expect("Database Error");
    c.is_none()
}

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
enum LoggedOutCommand {
    #[command(description = "display this text.")]
    Help,
    #[command(description = "create an account for your chat with the bot")]
    Start,
}

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
enum LoggedInCommand {
    #[command(description = "display this text.")]
    Help,
    #[command(description = "<RSS address> subscribe to an RSS feed")]
    Subscribe { link: String },
    #[command(description = "list feeds")]
    List,
    #[command(
        description = "<feed id> - unsubscribe from feed. Take the ids from the list command"
    )]
    Unsubscribe { feed_id: i64 },
    #[command(description = "delete my user account and all associated subscriptions")]
    DeleteAccount,
}

async fn create_chat(
    db: &DatabaseConnection,
    chat_id: i64,
) -> Result<chat::Model, Box<dyn Error + Send + Sync>> {
    let new_chat = chat::ActiveModel {
        id: ActiveValue::Set(chat_id),
        ..Default::default()
    };
    Ok(new_chat.insert(db).await?)
}

async fn process_logged_out_command(
    bot: Bot,
    msg: Message,
    cmd: LoggedOutCommand,
    db: DatabaseConnection,
) -> ResponseResult<()> {
    // commands for logged out users:
    // /help -> Send command list
    // /start -> Add chat to database
    match cmd {
        LoggedOutCommand::Help => {
            bot.send_message(msg.chat.id, LoggedOutCommand::descriptions().to_string())
                .await?;
        }
        LoggedOutCommand::Start => match create_chat(&db, msg.chat.id.0).await {
            Ok(new_chat) => {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "[{}] Registering your chat with the bot...Done.",
                        new_chat.created_at
                    ),
                )
                .await?;
            }
            Err(err) => {
                bot.send_message(
                    msg.chat.id,
                    format!("[{}] Error in registering new chat", err),
                )
                .await?;
            }
        },
    }
    Ok(())
}

/// Asynchronously validates and processes an RSS feed from a given URL.
///
/// This function fetches the content of the RSS feed from the specified URL, validates it,
/// and returns the parsed and validated `Channel` if successful.
///
/// # Arguments
///
/// * `link` - A reference to a `String` containing the URL of the RSS feed to be validated.
///
/// # Returns
///
/// Returns a `Result` where `Ok` contains the validated `Channel` if successful,
/// and `Err` contains an error implementing the `Error` trait in case of any issues.
///
/// # Errors
///
/// This function may return an error if:
/// - The HTTP request to fetch the feed content fails.
/// - The feed content cannot be parsed into a `Channel`.
/// - The parsed `Channel` fails the validation.
///
/// # Example
///
/// ```
/// use std::error::Error;
///
/// async fn main() -> Result<(), Box<dyn Error>> {
///     let url = "https://example.com/rss-feed.xml".to_string();
///     match validate_feed(&url).await {
///         Ok(channel) => {
///             println!("Feed validation successful: {:?}", channel);
///         }
///         Err(err) => {
///             eprintln!("Error while validating the feed: {}", err);
///         }
///     }
///     Ok(())
/// }
/// ```
///
async fn validate_feed(link: &String) -> Result<Channel, Box<dyn Error + Send + Sync>> {
    let content = reqwest::get(link).await?.bytes().await?;
    let mut channel = Channel::read_from(&content[..])?;
    channel.set_link(link);
    channel.validate()?;
    Ok(channel)
}

async fn create_feed(
    db: &DatabaseConnection,
    channel: &Channel,
    chat_id: i64,
) -> Result<feed::Model, Box<dyn Error + Send + Sync>> {
    let new_feed = feed::ActiveModel {
        chat_id: ActiveValue::Set(chat_id),
        title: ActiveValue::Set(channel.title.clone()),
        link: ActiveValue::Set(channel.link.clone()),
        ..Default::default()
    };
    Ok(new_feed.insert(db).await?)
}

async fn read_feed(
    db: &DatabaseConnection,
    chat_id: i64,
) -> Result<Vec<feed::Model>, Box<dyn Error + Send + Sync>> {
    Ok(entity::prelude::Feed::find()
        .filter(feed::Column::ChatId.eq(chat_id))
        .all(db)
        .await?)
}

async fn delete_feed(
    db: &DatabaseConnection,
    id: i64,
    chat_id: i64,
) -> Result<DeleteResult, Box<dyn Error + Send + Sync>> {
    Ok(entity::prelude::Feed::delete_many()
        .filter(feed::Column::ChatId.eq(chat_id))
        .filter(feed::Column::Id.eq(id))
        .exec(db)
        .await?)
}

async fn delete_chat(
    db: &DatabaseConnection,
    id: i64,
) -> Result<DeleteResult, Box<dyn Error + Send + Sync>> {
    Ok(entity::prelude::Chat::delete_by_id(id).exec(db).await?)
}

async fn process_command(
    bot: Bot,
    msg: Message,
    cmd: LoggedInCommand,
    db: DatabaseConnection,
) -> ResponseResult<()> {
    match cmd {
        LoggedInCommand::Help => {
            bot.send_message(msg.chat.id, LoggedInCommand::descriptions().to_string())
                .await?;
        }
        LoggedInCommand::Subscribe { link } => {
            let valid = validate_feed(&link).await;
            match valid {
                Ok(channel) => {
                    let new_feed = create_feed(&db, &channel, msg.chat.id.0).await;
                    match new_feed {
                        Ok(f) => {
                            bot.send_message(
                                msg.chat.id,
                                format!("Subscribed to feed:\n{}\n{}", f.title, f.link),
                            )
                            .await?;
                        }
                        Err(error) => {
                            bot.send_message(msg.chat.id, format!("Error: {}", error))
                                .await?;
                        }
                    }
                }
                Err(error) => {
                    bot.send_message(msg.chat.id, format!("Error: {}", error))
                        .await?;
                }
            }
        }
        LoggedInCommand::Unsubscribe { feed_id } => {
            let deleted = delete_feed(&db, feed_id, msg.chat.id.0).await;
            match deleted {
                Ok(delete_result) => {
                    bot.send_message(
                        msg.chat.id,
                        format!("Deleted {} feed", delete_result.rows_affected),
                    )
                    .await?;
                }
                Err(error) => {
                    bot.send_message(msg.chat.id, format!("Error: {}", error))
                        .await?;
                }
            }
        }
        LoggedInCommand::List => {
            // Retrieve and list the user's subscribed RSS feeds.
            let feeds = read_feed(&db, msg.chat.id.0).await;
            match feeds {
                Ok(feeds) => {
                    let feed_list: String = feeds
                        .iter()
                        .map(|feed| format!("{} - {}", feed.id, feed.title))
                        .collect::<Vec<String>>()
                        .join("\n");
                    bot.send_message(msg.chat.id, feed_list).await?;
                }
                Err(error) => {
                    bot.send_message(msg.chat.id, format!("Error: {}", error))
                        .await?;
                }
            }
        }
        LoggedInCommand::DeleteAccount => {
            let deleted = delete_chat(&db, msg.chat.id.0).await;
            match deleted {
                Ok(_delete_result) => {
                    bot.send_message(msg.chat.id, "Bye bye. Your account has been deleted.")
                        .await?;
                }
                Err(error) => {
                    bot.send_message(msg.chat.id, format!("Error: {}", error))
                        .await?;
                }
            }
        }
    }

    Ok(())
}
