use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "important")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub rowid: i64,
    #[sea_orm(unique)]
    pub id: String,
    pub content: String,
    pub timestamp_us: i64,
}

impl ActiveModelBehavior for ActiveModel {}
