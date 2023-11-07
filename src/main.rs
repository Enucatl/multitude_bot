use std::env;
use std::error::Error;
use std::fs;

use rss::validation::Validate;
use rss::Channel;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, Database, DatabaseConnection, DbErr, DeleteResult,
    EntityTrait, QueryFilter,
};
use teloxide::{
    dispatching::{HandlerExt, UpdateFilterExt, dialogue::GetChatId},
    dptree,
    payloads::SendMessageSetters,
    prelude::{
        Bot, Dispatcher, LoggingErrorHandler, Requester, ResponseResult, Update,
    },
    types::{CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, Message},
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
    let every_30_seconds = every(30)
        .seconds()
        .perform(|| async { println!("Every minute at 00 and 30 seconds") });
    tokio::spawn(every_30_seconds);

    let handler = dptree::entry()
        .branch(
            // Filter messages from users who are not in the DB "logged out"
            Update::filter_message()
                .filter_async(is_not_subscribed)
                .filter_command::<LoggedOutCommand>()
                .endpoint(process_logged_out_command),
        )
        .branch(
            Update::filter_message()
                .filter_command::<LoggedInCommand>()
                .endpoint(process_command),
        )
        .branch(
            Update::filter_callback_query().endpoint(handle_callback),
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

async fn handle_callback(
    bot: Bot,
    q: CallbackQuery,
    db: &DatabaseConnection,
) -> RequestResult<()> {
    // match q.data {
    //     Some(data) => {
    //         match FeedCommand::parse(&data, "") {
    //             Ok(command) => {
    //                 match command {
    //                     FeedCommand::Delete { feed_id } => {
    //                         bot.send_message(q.chat_id().unwrap(), format!("Deleted {feed_id}.")).await?;
    //                     }
    //                     FeedCommand::Exit => {
    //                         bot.send_message(q.chat_id().unwrap(), format!("All done thanks.")).await?;
    //                     }
    //                 }
    //             }
    //             Err(err) => {
    //                 bot.send_message(q.chat_id().unwrap(), format!("Error {}", err)).await?;
    //             }
    //         }
    //     }
    //     None => {
    //         bot.send_message(q.chat_id().unwrap(), format!("No data in callback query")).await?;
    //     }
    // }
    Ok(())
}

// Function to parse the callback data and extract the feed id
fn parse_callback_data(data: &str) -> Result<i64, Box<dyn Error + Send + Sync>> {
    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() == 2 {
        // The second part is the feed id
        let feed_id = parts[1].parse::<i64>()?;
        Ok(feed_id)
    } else {
        Err("Invalid callback data format".into())
    }
}

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "commands from the feed CRUD callback query interface"
)]
enum FeedCommand {
    #[command(description = "delete this feed")]
    Delete { feed_id: i64 },
    #[command(description = "Exit from the keyboard")]
    Exit,
}

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
enum LoggedOutCommand {
    #[command(description = "display this text.")]
    Help,
    #[command(description = "subscribe to this RSS feed")]
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
    #[command(description = "subscribe to this RSS feed")]
    Subscribe { link: String },
    #[command(description = "unsubscribe feed")]
    Unsubscribe,
    #[command(description = "delete my user account")]
    Delete,
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
) -> Result<DeleteResult, Box<dyn Error + Send + Sync>> {
    Ok(entity::prelude::Feed::delete_by_id(id).exec(db).await?)
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
                                format!("Feed is valid:\n{}\n{}", channel.title, channel.link),
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
        LoggedInCommand::Unsubscribe => {
            // Retrieve and list the user's subscribed RSS feeds.
            // Clicking deletes them from the table
            let feeds = read_feed(&db, msg.chat.id.0).await;
            match feeds {
                Ok(feeds) => {
                    let mut buttons: Vec<InlineKeyboardButton> = feeds
                        .iter()
                        .map(|f| {
                            let callback_data = FeedCommand::Delete(f.id);
                            InlineKeyboardButton::callback(
                                f.title.clone(),
                                callback_data.to_string(),
                            )
                        })
                        .collect();
                    buttons.push(InlineKeyboardButton::callback(
                        "Exit menu",
                        FeedCommand::Exit.to_string(),
                    ));
                    bot.send_message(msg.chat.id, "Currently registered feeds:")
                        .reply_markup(InlineKeyboardMarkup::new([buttons]))
                        .await?;
                }
                Err(error) => {
                    bot.send_message(msg.chat.id, format!("Error: {}", error))
                        .await?;
                }
            }
        }
        LoggedInCommand::Delete => {
            // Implement Delete command logic to delete a user account.
            // Perform user account deletion or provide instructions.
            bot.send_message(msg.chat.id, "Your account has been deleted.")
                .await?;
        }
    }

    Ok(())
}
