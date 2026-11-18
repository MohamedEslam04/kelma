//! Versioned schema migrations driven by `PRAGMA user_version`.
//!
//! Append new entries to `MIGRATIONS`; never edit an existing one.

use rusqlite::Connection;

use crate::{Error, Result};

#[cfg(test)]
pub(crate) const MIGRATIONS_LEN: usize = MIGRATIONS.len();

const MIGRATIONS: &[&str] = &[
    // v1: initial schema.
    //
    // No foreign key from messages to conversations on purpose: this is a
    // cache and a message may be stored before its conversation row is.
    "CREATE TABLE conversations (
         id               TEXT PRIMARY KEY NOT NULL,
         kind             TEXT NOT NULL,
         title            TEXT NOT NULL,
         team_id          TEXT,
         team_name        TEXT,
         last_activity_ms INTEGER NOT NULL,
         last_preview     TEXT NOT NULL,
         unread           INTEGER NOT NULL,
         favorite         INTEGER NOT NULL,
         read_horizon_ms  INTEGER NOT NULL
     );
     CREATE INDEX idx_conversations_last_activity ON conversations(last_activity_ms);
     CREATE INDEX idx_conversations_team ON conversations(team_id);

     CREATE TABLE messages (
         id                TEXT PRIMARY KEY NOT NULL,
         conversation_id   TEXT NOT NULL,
         parent_id         TEXT,
         sender_mri        TEXT NOT NULL,
         sender_name       TEXT NOT NULL,
         compose_ms        INTEGER NOT NULL,
         edited_ms         INTEGER,
         deleted           INTEGER NOT NULL,
         message_type      TEXT NOT NULL,
         content_html      TEXT NOT NULL,
         client_message_id TEXT,
         reactions_json    TEXT NOT NULL
     );
     CREATE INDEX idx_messages_conv_compose ON messages(conversation_id, compose_ms);
     CREATE INDEX idx_messages_parent ON messages(parent_id);

     CREATE TABLE users (
         mri          TEXT PRIMARY KEY NOT NULL,
         display_name TEXT NOT NULL,
         email        TEXT,
         avatar       BLOB,
         updated_ms   INTEGER NOT NULL
     );

     CREATE TABLE teams (
         id   TEXT PRIMARY KEY NOT NULL,
         name TEXT NOT NULL
     );

     CREATE TABLE kv (
         key   TEXT PRIMARY KEY NOT NULL,
         value TEXT NOT NULL
     );",
];

pub(crate) fn run(conn: &mut Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let supported = MIGRATIONS.len() as i64;
    if current > supported {
        return Err(Error::SchemaTooNew {
            found: current,
            supported,
        });
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        // PRAGMA does not accept bound parameters.
        tx.execute_batch(&format!("PRAGMA user_version = {}", i + 1))?;
        tx.commit()?;
    }
    Ok(())
}
