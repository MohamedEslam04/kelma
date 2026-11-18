//! Versioned schema migrations driven by `PRAGMA user_version`.
//!
//! Append new entries to `MIGRATIONS`; never edit an existing one.

use rusqlite::Connection;

use crate::{Error, Result};

#[cfg(test)]
pub(crate) const MIGRATIONS_LEN: usize = MIGRATIONS.len();

pub(crate) const MIGRATIONS: &[&str] = &[
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
    // v2: unsent drafts and the persistent send queue.
    //
    // Drafts get their own table rather than `draft:<conv>` kv entries: kv has
    // no timestamp to sort the Drafts view by, and listing by key prefix would
    // need LIKE escaping plus JSON in a crate that has no JSON dependency.
    "CREATE TABLE drafts (
         conversation_id TEXT PRIMARY KEY NOT NULL,
         text            TEXT NOT NULL,
         reply_to        TEXT,
         updated_ms      INTEGER NOT NULL
     );
     CREATE INDEX idx_drafts_updated ON drafts(updated_ms);

     CREATE TABLE outbox (
         client_message_id TEXT PRIMARY KEY NOT NULL,
         conversation_id   TEXT NOT NULL,
         html              TEXT NOT NULL,
         subject           TEXT,
         reply_to          TEXT,
         kind              TEXT NOT NULL,
         payload_path      TEXT,
         created_ms        INTEGER NOT NULL,
         attempts          INTEGER NOT NULL,
         state             TEXT NOT NULL,
         last_error        TEXT
     );
     CREATE INDEX idx_outbox_state_created ON outbox(state, created_ms);
     CREATE INDEX idx_outbox_conv ON outbox(conversation_id, created_ms);",
    // v3: messages saved with "Save this message".
    //
    // The server list (`48:saved`) is not available for every tenant, so kelma keeps its own
    // list. The sender and a text preview are copied so a Saved view needs no join and still
    // works after the message scrolls out of the cache.
    "CREATE TABLE saved_messages (
         conversation_id TEXT NOT NULL,
         message_id      TEXT NOT NULL,
         root_id         TEXT NOT NULL,
         sender_name     TEXT NOT NULL,
         preview         TEXT NOT NULL,
         saved_ms        INTEGER NOT NULL,
         PRIMARY KEY (conversation_id, message_id)
     );
     CREATE INDEX idx_saved_ms ON saved_messages(saved_ms);",
    // v4: the mentions property of a queued message (a JSON array string), so an @mention
    // survives the outbox like a quote reply's `reply_to` does.
    "ALTER TABLE outbox ADD COLUMN mentions TEXT;",
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
