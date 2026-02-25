use anyhow::{Context, Result};
use redis::{Commands, ConnectionLike};
use std::collections::HashMap;

/// Information about a Redis key
#[derive(Debug, Clone)]
pub struct KeyInfo {
    pub name: String,
    pub key_type: String,
    pub ttl: i64,
    pub size: i64,
    pub encoding: String,
}

/// A single stream entry
#[derive(Debug, Clone)]
pub struct StreamEntry {
    pub id: String,
    pub fields: Vec<(String, Vec<u8>)>,
}

/// The value of a Redis key, typed by its Redis data type
#[derive(Debug, Clone)]
pub enum RedisValue {
    String(Vec<u8>),
    List(Vec<Vec<u8>>),
    Set(Vec<Vec<u8>>),
    ZSet(Vec<(Vec<u8>, f64)>),
    Hash(Vec<(String, Vec<u8>)>),
    Stream(Vec<StreamEntry>),
    Unknown(String),
}

#[allow(dead_code)]
pub struct RedisClient {
    connection: redis::Connection,
    pub url: String,
    pub db: i64,
}

impl RedisClient {
    pub fn connect(url: &str) -> Result<Self> {
        let client = redis::Client::open(url)
            .with_context(|| format!("Failed to create Redis client for {}", url))?;
        let connection = client
            .get_connection()
            .with_context(|| format!("Failed to connect to {}", url))?;

        // Parse db number from URL (e.g., redis://host:port/3)
        let db = url
            .rsplit('/')
            .next()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);

        Ok(Self {
            connection,
            url: url.to_string(),
            db: db as i64,
        })
    }

    pub fn select_db(&mut self, db: i64) -> Result<()> {
        redis::cmd("SELECT")
            .arg(db)
            .exec(&mut self.connection)
            .with_context(|| format!("Failed to SELECT db {}", db))?;
        self.db = db;
        Ok(())
    }

    pub fn scan_keys(&mut self, pattern: &str) -> Result<Vec<String>> {
        let iter: redis::Iter<String> = redis::cmd("SCAN")
            .cursor_arg(0)
            .arg("MATCH")
            .arg(pattern)
            .arg("COUNT")
            .arg(1000)
            .clone()
            .iter(&mut self.connection)
            .context("Failed to SCAN keys")?;

        let mut keys: Vec<String> = iter.filter_map(|r| r.ok()).collect();
        keys.sort();
        Ok(keys)
    }

    pub fn get_key_info(&mut self, key: &str) -> Result<KeyInfo> {
        let key_type: String = redis::cmd("TYPE")
            .arg(key)
            .query(&mut self.connection)
            .unwrap_or_else(|_| "unknown".to_string());

        let ttl: i64 = self.connection.ttl(key).unwrap_or(-2);

        let size: i64 = redis::cmd("MEMORY")
            .arg("USAGE")
            .arg(key)
            .query(&mut self.connection)
            .unwrap_or(-1);

        let encoding: String = redis::cmd("OBJECT")
            .arg("ENCODING")
            .arg(key)
            .query(&mut self.connection)
            .unwrap_or_else(|_| "unknown".to_string());

        Ok(KeyInfo {
            name: key.to_string(),
            key_type,
            ttl,
            size,
            encoding,
        })
    }

    pub fn get_value(&mut self, key: &str) -> Result<RedisValue> {
        let key_type: String = redis::cmd("TYPE")
            .arg(key)
            .query(&mut self.connection)
            .unwrap_or_else(|_| "unknown".to_string());

        match key_type.as_str() {
            "string" => {
                let val: Vec<u8> = self.connection.get(key).context("Failed to GET")?;
                Ok(RedisValue::String(val))
            }
            "list" => {
                let vals: Vec<Vec<u8>> = self
                    .connection
                    .lrange(key, 0, -1)
                    .context("Failed to LRANGE")?;
                Ok(RedisValue::List(vals))
            }
            "set" => {
                let vals: Vec<Vec<u8>> = self
                    .connection
                    .smembers(key)
                    .context("Failed to SMEMBERS")?;
                Ok(RedisValue::Set(vals))
            }
            "zset" => {
                let vals: Vec<(Vec<u8>, f64)> = self
                    .connection
                    .zrange_withscores(key, 0, -1)
                    .context("Failed to ZRANGEBYSCORE")?;
                Ok(RedisValue::ZSet(vals))
            }
            "hash" => {
                let map: HashMap<String, Vec<u8>> = self
                    .connection
                    .hgetall(key)
                    .context("Failed to HGETALL")?;
                let mut pairs: Vec<(String, Vec<u8>)> = map.into_iter().collect();
                pairs.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(RedisValue::Hash(pairs))
            }
            "stream" => {
                let entries = self.get_stream_entries(key)?;
                Ok(RedisValue::Stream(entries))
            }
            other => Ok(RedisValue::Unknown(format!("Unsupported type: {}", other))),
        }
    }

    pub fn get_stream_entries(&mut self, key: &str) -> Result<Vec<StreamEntry>> {
        // XRANGE key - + COUNT 500
        let raw: Vec<redis::Value> = redis::cmd("XRANGE")
            .arg(key)
            .arg("-")
            .arg("+")
            .arg("COUNT")
            .arg(500)
            .query(&mut self.connection)
            .context("Failed to XRANGE")?;

        let mut entries = Vec::new();
        for entry_val in raw {
            if let redis::Value::Array(parts) = entry_val {
                if parts.len() >= 2 {
                    let id = match &parts[0] {
                        redis::Value::BulkString(b) => {
                            String::from_utf8_lossy(b).to_string()
                        }
                        _ => continue,
                    };

                    let mut fields = Vec::new();
                    if let redis::Value::Array(field_vals) = &parts[1] {
                        let mut i = 0;
                        while i + 1 < field_vals.len() {
                            let fname = match &field_vals[i] {
                                redis::Value::BulkString(b) => {
                                    String::from_utf8_lossy(b).to_string()
                                }
                                _ => {
                                    i += 2;
                                    continue;
                                }
                            };
                            let fval = match &field_vals[i + 1] {
                                redis::Value::BulkString(b) => b.clone(),
                                _ => Vec::new(),
                            };
                            fields.push((fname, fval));
                            i += 2;
                        }
                    }

                    entries.push(StreamEntry { id, fields });
                }
            }
        }

        Ok(entries)
    }

    pub fn delete_key(&mut self, key: &str) -> Result<()> {
        let _: () = self.connection
            .del(key)
            .context("Failed to DEL key")?;
        Ok(())
    }

    pub fn get_db_size(&mut self) -> Result<i64> {
        let size: i64 = redis::cmd("DBSIZE")
            .query(&mut self.connection)
            .unwrap_or(0);
        Ok(size)
    }

    #[allow(dead_code)]
    pub fn get_info_section(&mut self, section: &str) -> Result<String> {
        let info: String = redis::cmd("INFO")
            .arg(section)
            .query(&mut self.connection)
            .unwrap_or_default();
        Ok(info)
    }

    pub fn is_connected(&mut self) -> bool {
        self.connection.is_open()
    }

    // ─── Write operations ────────────────────────────────────

    pub fn set_string(&mut self, key: &str, value: &str) -> Result<()> {
        let _: () = self.connection.set(key, value).context("Failed to SET")?;
        Ok(())
    }

    pub fn hset(&mut self, key: &str, field: &str, value: &str) -> Result<()> {
        let _: () = self
            .connection
            .hset(key, field, value)
            .context("Failed to HSET")?;
        Ok(())
    }

    pub fn rpush(&mut self, key: &str, value: &str) -> Result<()> {
        let _: i64 = self
            .connection
            .rpush(key, value)
            .context("Failed to RPUSH")?;
        Ok(())
    }

    pub fn lset(&mut self, key: &str, index: i64, value: &str) -> Result<()> {
        let _: () = redis::cmd("LSET")
            .arg(key)
            .arg(index)
            .arg(value)
            .query(&mut self.connection)
            .context("Failed to LSET")?;
        Ok(())
    }

    pub fn sadd(&mut self, key: &str, member: &str) -> Result<()> {
        let _: i64 = self
            .connection
            .sadd(key, member)
            .context("Failed to SADD")?;
        Ok(())
    }

    pub fn zadd(&mut self, key: &str, score: f64, member: &str) -> Result<()> {
        let _: i64 = self
            .connection
            .zadd(key, member, score)
            .context("Failed to ZADD")?;
        Ok(())
    }

    pub fn xadd(&mut self, key: &str, field: &str, value: &str) -> Result<()> {
        let _: String = redis::cmd("XADD")
            .arg(key)
            .arg("*")
            .arg(field)
            .arg(value)
            .query(&mut self.connection)
            .context("Failed to XADD")?;
        Ok(())
    }

    pub fn set_ttl(&mut self, key: &str, ttl: i64) -> Result<()> {
        if ttl < 0 {
            let _: () = redis::cmd("PERSIST")
                .arg(key)
                .query(&mut self.connection)
                .context("Failed to PERSIST")?;
        } else {
            let _: () = self
                .connection
                .expire(key, ttl)
                .context("Failed to EXPIRE")?;
        }
        Ok(())
    }

    pub fn rename_key(&mut self, old_key: &str, new_key: &str) -> Result<()> {
        let _: () = self
            .connection
            .rename(old_key, new_key)
            .context("Failed to RENAME")?;
        Ok(())
    }
}
