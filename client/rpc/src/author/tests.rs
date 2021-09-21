// This file is part of Substrate.

// Copyright (C) 2017-2021 Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use super::*;

use assert_matches::assert_matches;
use codec::Encode;
use futures::executor;
use jsonrpsee::types::v2::{Response, RpcError};
use sc_transaction_pool::{BasicPool, FullChainApi};
use serde_json::value::to_raw_value;
use sp_core::{
	blake2_256,
	bytes::to_hex,
	crypto::{CryptoTypePublicPair, Pair, Public},
	ed25519,
	hexdisplay::HexDisplay,
	sr25519,
	testing::{ED25519, SR25519},
	H256,
};
use sp_keystore::testing::KeyStore;
use std::{mem, sync::Arc};
use substrate_test_runtime_client::{
	self,
	runtime::{Block, Extrinsic, SessionKeys, Transfer},
	AccountKeyring, Backend, Client, DefaultTestClientBuilderExt, TestClientBuilderExt,
};

fn uxt(sender: AccountKeyring, nonce: u64) -> Extrinsic {
	let tx =
		Transfer { amount: Default::default(), nonce, from: sender.into(), to: Default::default() };
	tx.into_signed_tx()
}

type FullTransactionPool = BasicPool<FullChainApi<Client<Backend>, Block>, Block>;

struct TestSetup {
	pub client: Arc<Client<Backend>>,
	pub keystore: Arc<KeyStore>,
	pub pool: Arc<FullTransactionPool>,
}

impl Default for TestSetup {
	fn default() -> Self {
		let keystore = Arc::new(KeyStore::new());
		let client_builder = substrate_test_runtime_client::TestClientBuilder::new();
		let client = Arc::new(client_builder.set_keystore(keystore.clone()).build());

		let spawner = sp_core::testing::TaskExecutor::new();
		let pool =
			BasicPool::new_full(Default::default(), true.into(), None, spawner, client.clone());
		TestSetup { client, keystore, pool }
	}
}

impl TestSetup {
	fn author(&self) -> Author<FullTransactionPool, Client<Backend>> {
		Author {
			client: self.client.clone(),
			pool: self.pool.clone(),
			keystore: self.keystore.clone(),
			deny_unsafe: DenyUnsafe::No,
			executor: SubscriptionTaskExecutor::default(),
		}
	}
}

#[tokio::test]
async fn author_submit_transaction_should_not_cause_error() {
	env_logger::init();
	let author = TestSetup::default().author();
	let api = author.into_rpc();
	let xt: Bytes = uxt(AccountKeyring::Alice, 1).encode().into();
	let extrinsic_hash: H256 = blake2_256(&xt).into();
	let params = to_raw_value(&[xt.clone()]).unwrap();
	let json = api.call("author_submitExtrinsic", Some(params)).await.unwrap();
	let response: Response<H256> = serde_json::from_str(&json).unwrap();

	assert_eq!(response.result, extrinsic_hash,);

	// Can't submit the same extrinsic twice
	let params_again = to_raw_value(&[xt]).unwrap();
	let json = api.call("author_submitExtrinsic", Some(params_again)).await.unwrap();
	let response: RpcError = serde_json::from_str(&json).unwrap();

	assert!(response.error.message.contains("Already imported"));
}

#[tokio::test]
async fn author_should_watch_extrinsic() {
	let api = TestSetup::default().author().into_rpc();

	let xt = {
		let xt_bytes = uxt(AccountKeyring::Alice, 0).encode();
		to_raw_value(&[to_hex(&xt_bytes, true)])
	}
	.unwrap();

	let (subscription_id, mut rx) =
		api.test_subscription("author_submitAndWatchExtrinsic", Some(xt)).await;
	let subscription_data = rx.next().await;

	let expected = Some(format!(
		// TODO: (dp) The `jsonrpc` version of this wraps the subscription ID in `"` – is this a problem? I think not.
		r#"{{"jsonrpc":"2.0","method":"author_submitAndWatchExtrinsic","params":{{"subscription":{},"result":"ready"}}}}"#,
		subscription_id,
	));
	assert_eq!(subscription_data, expected);

	// Replace the extrinsic and observe the subscription is notified.
	let (xt_replacement, xt_hash) = {
		let tx = Transfer {
			amount: 5,
			nonce: 0,
			from: AccountKeyring::Alice.into(),
			to: Default::default(),
		};
		let tx = tx.into_signed_tx().encode();
		let hash = blake2_256(&tx);

		(to_raw_value(&[to_hex(&tx, true)]).unwrap(), hash)
	};

	let json = api.call("author_submitExtrinsic", Some(xt_replacement)).await.unwrap();

	let expected = Some(format!(
		// TODO: (dp) The `jsonrpc` version of this wraps the subscription ID in `"` – is this a
		// problem? I think not.
		r#"{{"jsonrpc":"2.0","method":"author_submitAndWatchExtrinsic","params":{{"subscription":{},"result":{{"usurped":"0x{}"}}}}}}"#,
		subscription_id,
		HexDisplay::from(&xt_hash),
	));
	let subscription_data = rx.next().await;
	assert_eq!(subscription_data, expected);
}

// #[test]
// fn should_return_watch_validation_error() {
// 	// given
// 	let setup = TestSetup::default();
// 	let p = setup.author();

// 	let (subscriber, id_rx, _data) = jsonrpc_pubsub::typed::Subscriber::new_test("test");

// 	// when
// 	p.watch_extrinsic(
// 		Default::default(),
// 		subscriber,
// 		uxt(AccountKeyring::Alice, 179).encode().into(),
// 	);

// 	// then
// 	let res = executor::block_on(id_rx).unwrap();
// 	assert!(res.is_err(), "Expected the transaction to be rejected as invalid.");
// }

// #[test]
// fn should_return_pending_extrinsics() {
// 	let p = TestSetup::default().author();

// 	let ex = uxt(AccountKeyring::Alice, 0);
// 	executor::block_on(AuthorApi::submit_extrinsic(&p, ex.encode().into())).unwrap();
// 	assert_matches!(
// 		p.pending_extrinsics(),
// 		Ok(ref expected) if *expected == vec![Bytes(ex.encode())]
// 	);
// }

// #[test]
// fn should_remove_extrinsics() {
// 	let setup = TestSetup::default();
// 	let p = setup.author();

// 	let ex1 = uxt(AccountKeyring::Alice, 0);
// 	executor::block_on(p.submit_extrinsic(ex1.encode().into())).unwrap();
// 	let ex2 = uxt(AccountKeyring::Alice, 1);
// 	executor::block_on(p.submit_extrinsic(ex2.encode().into())).unwrap();
// 	let ex3 = uxt(AccountKeyring::Bob, 0);
// 	let hash3 = executor::block_on(p.submit_extrinsic(ex3.encode().into())).unwrap();
// 	assert_eq!(setup.pool.status().ready, 3);

// 	// now remove all 3
// 	let removed = p
// 		.remove_extrinsic(vec![
// 			hash::ExtrinsicOrHash::Hash(hash3),
// 			// Removing this one will also remove ex2
// 			hash::ExtrinsicOrHash::Extrinsic(ex1.encode().into()),
// 		])
// 		.unwrap();

// 	assert_eq!(removed.len(), 3);
// }

// #[test]
// fn should_insert_key() {
// 	let setup = TestSetup::default();
// 	let p = setup.author();

// 	let suri = "//Alice";
// 	let key_pair = ed25519::Pair::from_string(suri, None).expect("Generates keypair");
// 	p.insert_key(
// 		String::from_utf8(ED25519.0.to_vec()).expect("Keytype is a valid string"),
// 		suri.to_string(),
// 		key_pair.public().0.to_vec().into(),
// 	)
// 	.expect("Insert key");

// 	let public_keys = SyncCryptoStore::keys(&*setup.keystore, ED25519).unwrap();

// 	assert!(public_keys
// 		.contains(&CryptoTypePublicPair(ed25519::CRYPTO_ID, key_pair.public().to_raw_vec())));
// }

// #[test]
// fn should_rotate_keys() {
// 	let setup = TestSetup::default();
// 	let p = setup.author();

// 	let new_public_keys = p.rotate_keys().expect("Rotates the keys");

// 	let session_keys =
// 		SessionKeys::decode(&mut &new_public_keys[..]).expect("SessionKeys decode successfully");

// 	let ed25519_public_keys = SyncCryptoStore::keys(&*setup.keystore, ED25519).unwrap();
// 	let sr25519_public_keys = SyncCryptoStore::keys(&*setup.keystore, SR25519).unwrap();

// 	assert!(ed25519_public_keys
// 		.contains(&CryptoTypePublicPair(ed25519::CRYPTO_ID, session_keys.ed25519.to_raw_vec())));
// 	assert!(sr25519_public_keys
// 		.contains(&CryptoTypePublicPair(sr25519::CRYPTO_ID, session_keys.sr25519.to_raw_vec())));
// }

// #[test]
// fn test_has_session_keys() {
// 	let setup = TestSetup::default();
// 	let p = setup.author();

// 	let non_existent_public_keys =
// 		TestSetup::default().author().rotate_keys().expect("Rotates the keys");

// 	let public_keys = p.rotate_keys().expect("Rotates the keys");
// 	let test_vectors = vec![
// 		(public_keys, Ok(true)),
// 		(vec![1, 2, 3].into(), Err(Error::InvalidSessionKeys)),
// 		(non_existent_public_keys, Ok(false)),
// 	];

// 	for (keys, result) in test_vectors {
// 		assert_eq!(
// 			result.map_err(|e| mem::discriminant(&e)),
// 			p.has_session_keys(keys).map_err(|e| mem::discriminant(&e)),
// 		);
// 	}
// }

// #[test]
// fn test_has_key() {
// 	let setup = TestSetup::default();
// 	let p = setup.author();

// 	let suri = "//Alice";
// 	let alice_key_pair = ed25519::Pair::from_string(suri, None).expect("Generates keypair");
// 	p.insert_key(
// 		String::from_utf8(ED25519.0.to_vec()).expect("Keytype is a valid string"),
// 		suri.to_string(),
// 		alice_key_pair.public().0.to_vec().into(),
// 	)
// 	.expect("Insert key");
// 	let bob_key_pair = ed25519::Pair::from_string("//Bob", None).expect("Generates keypair");

// 	let test_vectors = vec![
// 		(alice_key_pair.public().to_raw_vec().into(), ED25519, Ok(true)),
// 		(alice_key_pair.public().to_raw_vec().into(), SR25519, Ok(false)),
// 		(bob_key_pair.public().to_raw_vec().into(), ED25519, Ok(false)),
// 	];

// 	for (key, key_type, result) in test_vectors {
// 		assert_eq!(
// 			result.map_err(|e| mem::discriminant(&e)),
// 			p.has_key(
// 				key,
// 				String::from_utf8(key_type.0.to_vec()).expect("Keytype is a valid string"),
// 			)
// 			.map_err(|e| mem::discriminant(&e)),
// 		);
// 	}
// }
