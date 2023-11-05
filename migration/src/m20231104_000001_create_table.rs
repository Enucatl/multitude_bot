use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Chat::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Chat::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Chat::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Feed::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Feed::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Feed::ChatId).big_integer().not_null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("ForeignKey-Feed-Chat")
                            .from(Feed::Table, Feed::ChatId)
                            .to(Chat::Table, Chat::Id),
                    )
                    .col(ColumnDef::new(Feed::Url).string().not_null())
                    .col(
                        ColumnDef::new(Feed::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Feed::UpdatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Feed::Table).to_owned())
            .await?;

        manager
            .drop_table(Table::drop().table(Chat::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Chat {
    Table,
    Id,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Feed {
    Table,
    Id,
    ChatId,
    Url,
    CreatedAt,
    UpdatedAt,
}
