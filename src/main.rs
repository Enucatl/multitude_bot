use std::env;
use std::fs;

use sea_orm;
use sea_orm::{ActiveModelTrait, EntityTrait};
use teloxide::{prelude::*, types::Message, utils::command::BotCommands};
use tokio_schedule::{every, Job};
use urlencoding::encode;

use entity::{chat, feed};
use migration::{Migrator, MigratorTrait};

const TELOXIDE_TOKEN_PATH: &str = "/run/secrets/teloxide_token";

async fn db_connect() -> Result<sea_orm::DatabaseConnection, sea_orm::DbErr> {
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
    let db = sea_orm::Database::connect(&db_url).await?;
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

    let handler = Update::filter_message()
        .branch(
            // Filter messages from users who are not in the DB "logged out"
            dptree::entry()
                .filter_async(is_not_subscribed)
                .filter_command::<LoggedOutCommand>()
                .endpoint(process_logged_out_command)
        )
        .branch(
            dptree::entry()
                .filter_command::<LoggedInCommand>()
                .endpoint(process_command),
        )
        .branch(
            // Handle other messages or actions here
            dptree::filter(|msg: Message| msg.chat.is_group() || msg.chat.is_supergroup())
                .endpoint(|bot: Bot, msg: Message| async move {
                    log::info!("Received a message from a group chat.");
                    bot.send_message(msg.chat.id, "This is a group chat.")
                        .await?;
                    Ok(())
                }),
        );

    // Your dispatcher setup and configuration here...

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

async fn is_not_subscribed(msg: Message, db: sea_orm::DatabaseConnection) -> bool {
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
    Subscribe { url: String },
    #[command(description = "list my subscriptions")]
    List,
    #[command(description = "unsubscribe feed")]
    Unsubscribe { feed_id: i64 },
    #[command(description = "delete my user account")]
    Delete,
}

async fn process_logged_out_command(
    bot: Bot,
    msg: Message,
    cmd: LoggedOutCommand,
    db: sea_orm::DatabaseConnection,
) -> ResponseResult<()> {
    match cmd {
        LoggedOutCommand::Help => {
            bot.send_message(msg.chat.id, LoggedOutCommand::descriptions().to_string())
                .await?;
        }
        LoggedOutCommand::Start => {
            let new_chat = chat::ActiveModel {
                id: sea_orm::ActiveValue::Set(msg.chat.id.0),
                ..Default::default()
            };
            let new_chat: chat::Model = new_chat.insert(&db).await.expect("DatabaseError");
            bot.send_message(msg.chat.id, "Registering your chat with the bot...Done.")
                .await?;
        }
    }

    Ok(())
}
async fn process_command(
    bot: Bot,
    msg: Message,
    cmd: LoggedInCommand,
    db: sea_orm::DatabaseConnection,
) -> ResponseResult<()> {
    match cmd {
        LoggedInCommand::Help => {
            bot.send_message(msg.chat.id, LoggedInCommand::descriptions().to_string())
                .await?;
        }
        LoggedInCommand::Subscribe { url } => {
            // Implement Subscribe command logic to subscribe to an RSS feed.
            // You can use the provided 'url' to determine which feed to subscribe to.

            bot.send_message(msg.chat.id, format!("Subscribed to: {}", url))
                .await?;
        }
        LoggedInCommand::List => {
            // Implement List command logic to list a user's subscriptions.
            // Retrieve and list the user's subscribed RSS feeds.
            bot.send_message(msg.chat.id, "Here is your list of subscriptions: ...")
                .await?;
        }
        LoggedInCommand::Unsubscribe { feed_id } => {
            // Implement Unsubscribe command logic to unsubscribe from a feed.
            // Use the 'feed_id' to identify the feed to unsubscribe from.
            bot.send_message(msg.chat.id, format!("Unsubscribed from feed: {}", feed_id))
                .await?;
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
