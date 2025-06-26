use super::{
    get_from_db_or_txn, get_message, make_fid_key, make_user_key, read_fid_key,
    store::{Store, StoreDef},
    IntoU8, MessagesPage, StoreEventHandler, TS_HASH_LENGTH,
};
use crate::core::error::HubError;
use crate::proto::message_data::Body;
use crate::proto::{self, HubEvent, HubEventType, MergeUserNameProofBody, Message, MessageType};
use crate::storage::constants::{RootPrefix, UserPostfix};
use crate::storage::db::PageOptions;
use crate::storage::db::{RocksDB, RocksDbTransactionBatch};
use crate::storage::util;
use std::sync::Arc;

#[derive(Clone)]
pub struct UsernameProofStoreDef {
    prune_size_limit: u32,
}

impl StoreDef for UsernameProofStoreDef {
    #[inline]
    fn postfix(&self) -> u8 {
        UserPostfix::UsernameProofMessage.as_u8()
    }

    #[inline]
    fn add_message_type(&self) -> u8 {
        MessageType::UsernameProof.into_u8()
    }

    #[inline]
    fn make_add_key(&self, message: &Message) -> Result<Vec<u8>, HubError> {
        if message.data.is_none() {
            return Err(HubError {
                code: "bad_request.validation_failure".to_string(),
                message: "Message data is missing".to_string(),
            });
        }

        let data = message.data.as_ref().unwrap();
        if data.body.is_none() {
            return Err(HubError {
                code: "bad_request.validation_failure".to_string(),
                message: "Message body is missing".to_string(),
            });
        }

        let name = match &data.body {
            Some(Body::UsernameProofBody(body)) => &body.name,
            _ => {
                return Err(HubError {
                    code: "bad_request.validation_failure".to_string(),
                    message: "Message body is missing".to_string(),
                })
            }
        };

        Ok(Self::make_username_proof_by_fid_key(
            message.data.as_ref().unwrap().fid,
            name,
        ))
    }

    #[inline]
    fn remove_type_supported(&self) -> bool {
        false
    }

    #[inline]
    fn compact_state_message_type(&self) -> u8 {
        MessageType::None as u8
    }

    #[inline]
    fn is_compact_state_type(&self, _message: &Message) -> bool {
        false
    }

    #[inline]
    fn make_remove_key(&self, _message: &Message) -> Result<Vec<u8>, HubError> {
        Err(HubError {
            code: "bad_request.validation_failure".to_string(),
            message: "Remove not supported".to_string(),
        })
    }

    #[inline]
    fn make_compact_state_add_key(&self, _message: &Message) -> Result<Vec<u8>, HubError> {
        Err(HubError {
            code: "bad_request.invalid_param".to_string(),
            message: "Username Proof Store doesn't support compact state".to_string(),
        })
    }

    #[inline]
    fn make_compact_state_prefix(&self, _fid: u64) -> Result<Vec<u8>, HubError> {
        Err(HubError {
            code: "bad_request.invalid_param".to_string(),
            message: "Username Proof Store doesn't support compact state".to_string(),
        })
    }

    #[inline]
    fn is_add_type(&self, message: &Message) -> bool {
        if message.data.is_none() {
            return false;
        }
        let data = message.data.as_ref().unwrap();
        message.signature_scheme == proto::SignatureScheme::Ed25519 as i32
            && data.r#type == MessageType::UsernameProof.into_u8() as i32
            && data.body.is_some()
    }

    #[inline]
    fn build_secondary_indices(
        &self,
        txn: &mut RocksDbTransactionBatch,
        _ts_hash: &[u8; TS_HASH_LENGTH],
        message: &Message,
    ) -> Result<(), HubError> {
        if message.data.is_none() {
            return Err(HubError {
                code: "bad_request.validation_failure".to_string(),
                message: "Message data is missing".to_string(),
            });
        }

        let data = message.data.as_ref().unwrap();
        if let Some(Body::UsernameProofBody(body)) = &data.body {
            if body.name.len() == 0 {
                return Err(HubError {
                    code: "bad_request.invalid_param".to_string(),
                    message: "name empty".to_string(),
                });
            }

            let by_name_key = Self::make_username_proof_by_name_key(&body.name);
            txn.put(
                by_name_key,
                make_fid_key(message.data.as_ref().unwrap().fid),
            );
            Ok(())
        } else {
            Err(HubError {
                code: "bad_request.validation_failure".to_string(),
                message: "Message body is missing or incorrect".to_string(),
            })
        }
    }

    #[inline]
    fn delete_secondary_indices(
        &self,
        txn: &mut RocksDbTransactionBatch,
        _ts_hash: &[u8; TS_HASH_LENGTH],
        message: &Message,
    ) -> Result<(), HubError> {
        if message.data.is_none() {
            return Err(HubError {
                code: "bad_request.validation_failure".to_string(),
                message: "Message data is missing".to_string(),
            });
        }

        let data = message.data.as_ref().unwrap();
        if let Some(Body::UsernameProofBody(body)) = &data.body {
            if body.name.len() == 0 {
                return Err(HubError {
                    code: "bad_request.invalid_param".to_string(),
                    message: "name empty".to_string(),
                });
            }

            let by_name_key = Self::make_username_proof_by_name_key(&body.name);
            txn.delete(by_name_key);
            Ok(())
        } else {
            Err(HubError {
                code: "bad_request.validation_failure".to_string(),
                message: "Message data body is missing or incorrect".to_string(),
            })
        }
    }

    fn get_merge_conflicts(
        &self,
        db: &RocksDB,
        txn: &mut RocksDbTransactionBatch,
        message: &Message,
        ts_hash: &[u8; TS_HASH_LENGTH],
    ) -> Result<Vec<Message>, HubError> {
        if message.data.is_none() {
            return Err(HubError {
                code: "bad_request.validation_failure".to_string(),
                message: "Message data is missing".to_string(),
            });
        }

        let data = message.data.as_ref().unwrap();
        let name = match &data.body {
            Some(Body::UsernameProofBody(body)) => &body.name,
            _ => {
                return Err(HubError {
                    code: "bad_request.validation_failure".to_string(),
                    message: "Message data body is missing".to_string(),
                })
            }
        };

        let mut conflicts = Vec::new();
        let by_name_key = Self::make_username_proof_by_name_key(name);

        let fid_result = get_from_db_or_txn(db, txn, by_name_key.as_slice());
        if let Ok(Some(fid_bytes)) = fid_result {
            let fid = read_fid_key(&fid_bytes, 0);
            if fid > 0 {
                let existing_add_key = Self::make_username_proof_by_fid_key(fid, name);
                if let Ok(existing_message_ts_hash) =
                    get_from_db_or_txn(db, txn, existing_add_key.as_slice())
                {
                    if let Ok(Some(existing_message)) = get_message(
                        db,
                        txn,
                        fid,
                        self.postfix(),
                        &util::vec_to_u8_24(&existing_message_ts_hash)?,
                    ) {
                        let message_compare = self.message_compare(
                            self.add_message_type(),
                            &existing_message_ts_hash.unwrap().to_vec(),
                            self.add_message_type(),
                            &ts_hash.to_vec(),
                        );

                        if message_compare > 0 {
                            return Err(HubError {
                                code: "bad_request.conflict".to_string(),
                                message: "message conflicts with a more recent add".to_string(),
                            });
                        }
                        if message_compare == 0 {
                            return Err(HubError {
                                code: "bad_request.duplicate".to_string(),
                                message: "message has already been merged".to_string(),
                            });
                        }
                        conflicts.push(existing_message);
                    }
                }
            }
        }

        Ok(conflicts)
    }

    #[inline]
    fn remove_message_type(&self) -> u8 {
        MessageType::None.into_u8()
    }

    #[inline]
    fn is_remove_type(&self, _message: &Message) -> bool {
        false
    }

    #[inline]
    fn get_prune_size_limit(&self) -> u32 {
        self.prune_size_limit
    }

    #[inline]
    fn revoke_event_args(&self, message: &Message) -> HubEvent {
        let username_proof_body = match &message.data {
            Some(message_data) => match &message_data.body {
                Some(Body::UsernameProofBody(username_proof_body)) => {
                    Some(username_proof_body.clone())
                }
                _ => None,
            },
            _ => None,
        };

        HubEvent::from(
            HubEventType::MergeUsernameProof,
            proto::hub_event::Body::MergeUsernameProofBody(MergeUserNameProofBody {
                username_proof: None,
                deleted_username_proof: username_proof_body,
                username_proof_message: None,
                deleted_username_proof_message: Some(message.clone()),
            }),
        )
    }

    fn merge_event_args(&self, message: &Message, merge_conflicts: Vec<Message>) -> HubEvent {
        let username_proof_body = match &message.data {
            Some(message_data) => match &message_data.body {
                Some(Body::UsernameProofBody(username_proof_body)) => {
                    Some(username_proof_body.clone())
                }
                _ => None,
            },
            _ => None,
        };

        let (deleted_proof_body, deleted_message) = if merge_conflicts.len() > 0 {
            match &merge_conflicts[0].data {
                Some(message_data) => match &message_data.body {
                    Some(Body::UsernameProofBody(username_proof_body)) => (
                        Some(username_proof_body.clone()),
                        Some(merge_conflicts[0].clone()),
                    ),
                    _ => (None, None),
                },
                _ => (None, None),
            }
        } else {
            (None, None)
        };

        HubEvent::from(
            HubEventType::MergeUsernameProof,
            proto::hub_event::Body::MergeUsernameProofBody(MergeUserNameProofBody {
                username_proof: username_proof_body,
                deleted_username_proof: deleted_proof_body,
                username_proof_message: Some(message.clone()),
                deleted_username_proof_message: deleted_message,
            }),
        )
    }

    #[inline]
    fn prune_event_args(&self, message: &Message) -> HubEvent {
        self.revoke_event_args(message)
    }
}

impl UsernameProofStoreDef {
    #[inline]
    fn make_username_proof_by_name_key(name: &Vec<u8>) -> Vec<u8> {
        let mut key = Vec::with_capacity(1 + name.len());

        key.push(RootPrefix::UserNameProofByName as u8);
        key.extend(name);

        key
    }

    #[inline]
    fn make_username_proof_by_fid_key(fid: u64, name: &Vec<u8>) -> Vec<u8> {
        let mut key = Vec::with_capacity(1 + 4 + 1 + name.len());

        key.extend_from_slice(&make_user_key(fid));
        key.push(UserPostfix::UserNameProofAdds.as_u8());
        key.extend(name);

        key
    }
}

pub struct UsernameProofStore {}

impl UsernameProofStore {
    pub fn new(
        db: Arc<RocksDB>,
        store_event_handler: Arc<StoreEventHandler>,
        prune_size_limit: u32,
    ) -> Store<UsernameProofStoreDef> {
        Store::new_with_store_def(
            db,
            store_event_handler,
            UsernameProofStoreDef { prune_size_limit },
        )
    }

    pub fn get_username_proof(
        store: &Store<UsernameProofStoreDef>,
        name: &Vec<u8>,
    ) -> Result<Option<Message>, HubError> {
        let by_name_key = UsernameProofStoreDef::make_username_proof_by_name_key(name);
        let fid_result = store.db().get(by_name_key.as_slice())?;
        if fid_result.is_none() {
            return Err(HubError {
                code: "not_found".to_string(),
                message: format!(
                    "NotFound: Username proof not found for name {}",
                    String::from_utf8_lossy(name)
                ),
            });
        }

        let fid = read_fid_key(&fid_result.unwrap(), 0);
        let partial_message = Message {
            data: Some(proto::MessageData {
                fid,
                body: Some(Body::UsernameProofBody(proto::UserNameProof {
                    name: name.clone(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        };

        store.get_add(&partial_message)
    }

    pub fn get_username_proofs_by_fid(
        store: &Store<UsernameProofStoreDef>,
        fid: u64,
        page_options: &PageOptions,
    ) -> Result<MessagesPage, HubError> {
        store.get_adds_by_fid::<fn(&Message) -> bool>(fid, page_options, None)
    }

    pub fn get_username_proof_by_fid_and_name(
        store: &Store<UsernameProofStoreDef>,
        name: &Vec<u8>,
        fid: u64,
    ) -> Result<Option<Message>, HubError> {
        let partial_message = Message {
            data: Some(proto::MessageData {
                fid,
                body: Some(Body::UsernameProofBody(proto::UserNameProof {
                    name: name.clone(),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        };

        store.get_add(&partial_message)
    }
}
