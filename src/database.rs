// Copyright 2017-2019 Parity Technologies (UK) Ltd.
// This file is part of substrate-archive.

// substrate-archive is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// substrate-archive is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with substrate-archive.  If not, see <http://www.gnu.org/licenses/>.

//! IO for the PostgreSQL database connected to Substrate Archive Node

pub mod models;
pub mod schema;
pub mod db_middleware;

use log::*;
use failure::Error;
use futures::future::{Future, join_all};
use runtime_primitives::{traits::Block, generic::UncheckedExtrinsic};
use diesel::{prelude::*, pg::PgConnection};
use codec::{Encode, Decode};
use runtime_primitives::traits::Header;
use dotenv::dotenv;
use chrono::offset::{Utc, TimeZone};

use std::{
    env,
    convert::TryFrom,
    sync::{Arc, Mutex}
};

use crate::{
    error::Error as ArchiveError,
    types::{Data, System, BasicExtrinsic, ExtractCall},
    database::{
        models::{InsertBlock, InsertInherent, Inherents, Blocks},
        schema::{blocks, inherents}
    },
};

use self::db_middleware::AsyncDiesel;

/// Database object which communicates with Diesel in a (psuedo)asyncronous way
/// via `AsyncDiesel`
pub struct Database {
    db: AsyncDiesel<PgConnection>,
}

impl Database {

    /// Connect to the database
    pub fn new() -> Result<Self, ArchiveError> {
        dotenv().ok();
        let database_url = env::var("DATABASE_URL")?;
        let db = AsyncDiesel::new(&database_url)?;
        Ok(Self { db })
    }

    // TODO: make async
    pub fn insert<T>(&self, data: &Data<T>) -> impl Future<Item = (), Error = ArchiveError>
    where
        T: System
    {
        match &data {
            Data::Block(block) => {
                let header = &block.block.header;
                let extrinsics = block.block.extrinsics();
                info!("HASH: {:X?}", header.hash().as_ref());
                self.db.run(move |conn| {
                    diesel::insert_into(blocks::table)
                        .values( InsertBlock {
                            parent_hash: header.parent_hash().as_ref(),
                            hash: header.hash().as_ref(),
                            block: &(*header.number()).into(),
                            state_root: header.state_root().as_ref(),
                            extrinsics_root: header.extrinsics_root().as_ref(),
                            time: None
                        })
                        .execute(&conn)
                        .map_err(|e| e.into())
                }).and_then (|res| {
                    let inherents = Vec::new();
                    for (idx, e) in extrinsics.iter().enumerate() {
                        //TODO possibly redundant operation
                        let encoded = e.encode();
                        let decoded: BasicExtrinsic<T> =
                            UncheckedExtrinsic::decode(&mut encoded.as_slice()).expect("Tempt Expect -- Decode Ext Failed!");
                        let (module, call) = decoded.function.extract_call();
                        let (fn_name, params) = call.function().expect("Temp Expect -- Function in Trait Failed 96 database");
                        inherents.push(
                            InsertInherent {
                                hash: header.hash().as_ref(),
                                block: &(*header.number()).into(),
                                module: &String::from(module),
                                call: &fn_name,
                                parameters: Some(params),
                                success: &true, // TODO not always true
                                in_index: &(i32::try_from(idx).expect("Temp Expect --- TryFrom failed 105 database"))
                            }
                        );
                    }
                    self.db.run(move |conn| {
                        diesel::insert_into(inherents::table)
                            .values(&inherents)
                            .execute(&conn)
                            .map_err(|e| e.into())
                    })
                })
            },
            Data::Storage(data, from, hash) => {
                use self::schema::blocks::dsl::{blocks, time};
                let unix_time: i64 = Decode::decode(&mut data.0.as_slice()).expect("Decoding failed");
                let date_time = Utc.timestamp_millis(unix_time); // panics if time is incorrect
                self.db.run(move |conn| {
                    diesel::update(blocks.find(hash.as_ref()))
                        .set(time.eq(Some(&date_time)))
                        .execute(&conn)
                        .map_err(|e| e.into())
                }).and_then(|res| {
                    futures::future::ok(())
                })
            }
            _ => {
            }
        }
    }
}
