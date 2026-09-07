//! 显式原子升级；调用者必须持有唯一数据库入口创建的立即事务。

use super::{SessionError, schema, schema_v2};
use rusqlite::Connection;

pub(super) fn v1(db: &Connection, config: &str) -> Result<(), SessionError> {
    if db.is_autocommit() || schema::version(db)? != 1 {
        return Err(SessionError::UnsupportedSchema);
    }
    schema::validate(db, config)?;
    schema_v2::initialize(db)?;
    schema::validate(db, config)
}

pub(super) fn v2(db: &Connection, config: &str) -> Result<(), SessionError> {
    if db.is_autocommit() || schema::version(db)? != 2 {
        return Err(SessionError::UnsupportedSchema);
    }
    schema::validate(db, config)?;
    super::totp::schema::initialize(db)?;
    schema::validate(db, config)
}

pub(super) fn v3(db: &Connection, config: &str) -> Result<(), SessionError> {
    if db.is_autocommit() || schema::version(db)? != 3 {
        return Err(SessionError::UnsupportedSchema);
    }
    schema::validate(db, config)?;
    super::recovery::schema::initialize(db)?;
    schema::validate(db, config)
}
