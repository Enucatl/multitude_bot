use std::env;
use std::fs;

use sea_orm;
use teloxide::{prelude::*, utils::command::BotCommands};
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

    Command::repl(bot, answer).await;
}

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
enum Command {
    #[command(description = "display this text.")]
    Help,
    #[command(description = "handle a username.")]
    Username(String),
    #[command(description = "handle a username and an age.", parse_with = "split")]
    UsernameAndAge { username: String, age: u8 },
}

async fn answer(bot: Bot, msg: Message, cmd: Command) -> ResponseResult<()> {
    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?
        }
        Command::Username(username) => {
            bot.send_message(msg.chat.id, format!("Your username is @{username}."))
                .await?
        }
        Command::UsernameAndAge { username, age } => {
            bot.send_message(
                msg.chat.id,
                format!("Your username is @{username} and age is {age}."),
            )
            .await?
        }
    };

    Ok(())
}
