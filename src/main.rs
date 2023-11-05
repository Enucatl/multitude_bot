use std::env;
use std::fs;

use sea_orm;
use teloxide::{prelude::*, utils::command::BotCommands, types::Message};
use tokio_schedule::{every, Job};
use urlencoding::encode;

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
            // Filter messages that are commands
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(process_command)
        )
        .branch(
            // Handle other messages or actions here
            dptree::filter(|msg: Message| msg.chat.is_group() || msg.chat.is_supergroup())
                .endpoint(|bot: Bot, msg: Message| async move {
                    log::info!("Received a message from a group chat.");
                    bot.send_message(msg.chat.id, "This is a group chat.").await?;
                    Ok(())
                }),
        );

    // Your dispatcher setup and configuration here...

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![])
        .default_handler(|upd| async move {
            log::warn!("Unhandled update: {:?}", upd);
        })
        .error_handler(LoggingErrorHandler::with_custom_text("An error has occurred in the dispatcher"))
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

}

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
enum Command {
    #[command(description = "display this text.")]
    Help,
    #[command(description = "start using this bot")]
    Start,
    #[command(description = "subscribe to this RSS feed")]
    Subscribe { url: String },
    #[command(description = "list my subscriptions")]
    List,
    #[command(description = "unsubscribe feed")]
    Unsubscribe { feed_id: i64 },
    #[command(description = "delete my user account")]
    Delete,
}

async fn process_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
) -> ResponseResult<()> {
    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
        }
        Command::Start => {
            // Implement Start command logic for when a user starts using the bot.
            // Provide a welcome message or instructions for starting.
            // let db = db_connect().await?;
            // // Find by primary key
            // log.info!("{}", msg.chat.id);
            // let chat: Option<chat::Model> = Chat::find_by_id(msg.chat.id).one(db).await?;
            
            bot.send_message(msg.chat.id, "Welcome! You have started using the bot.").await?;
        }
        Command::Subscribe { url } => {
            // Implement Subscribe command logic to subscribe to an RSS feed.
            // You can use the provided 'url' to determine which feed to subscribe to.
            bot.send_message(msg.chat.id, format!("Subscribed to: {}", url)).await?;
        }
        Command::List => {
            // Implement List command logic to list a user's subscriptions.
            // Retrieve and list the user's subscribed RSS feeds.
            bot.send_message(msg.chat.id, "Here is your list of subscriptions: ...").await?;
        }
        Command::Unsubscribe { feed_id } => {
            // Implement Unsubscribe command logic to unsubscribe from a feed.
            // Use the 'feed_id' to identify the feed to unsubscribe from.
            bot.send_message(msg.chat.id, format!("Unsubscribed from feed: {}", feed_id)).await?;
        }
        Command::Delete => {
            // Implement Delete command logic to delete a user account.
            // Perform user account deletion or provide instructions.
            bot.send_message(msg.chat.id, "Your account has been deleted.").await?;
        }
    }

    Ok(())
}
