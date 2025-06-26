use crate::core::validations::error::ValidationError;
use crate::proto::{self, FarcasterNetwork, VerificationAddAddressBody};
use alloy_dyn_abi::TypedData;
use alloy_provider::Provider;
use alloy_transport::Transport;
use eth_signature_verifier::Verification;
use serde::Serialize;
use serde_json::{json, Value};

const EIP_712_FARCASTER_VERIFICATION_CLAIM_CHAIN_IDS: [u16; 5] = [0, 1, 5, 10, 420];
const FNAME_SIGNER_ADDRESS: alloy_primitives::Address =
    alloy_primitives::address!("Bc5274eFc266311015793d89E9B591fa46294741");

fn eip_712_farcaster_verification_claim() -> Value {
    json!({
      "EIP712Domain": [
        {
            "name": "name",
            "type": "string"
        },
        {
            "name": "version",
            "type": "string"
        },
        {
            "name": "chainId",
            "type": "uint256"
        },
        {
            "name": "verifyingContract",
            "type": "address"
        }
      ],
      "VerificationClaim": [
        {
          "name": "fid",
          "type": "uint256",
        },
        {
          "name": "address",
          "type": "address",
        },
        {
          "name": "blockHash",
          "type": "bytes32",
        },
        {
          "name": "network",
          "type": "uint8",
        },
      ],
    })
}

pub fn eip_712_domain() -> Value {
    json!({
        "EIP712Domain": [
            {
                "name": "name",
                "type": "string"
            },
            {
                "name": "version",
                "type": "string"
            },
            {
                "name": "chainId",
                "type": "uint256"
            },
            {
                "name": "verifyingContract",
                "type": "address"
            }
        ],
        "UserNameProof": [
            { "name": "name", "type": "string" },
            { "name": "timestamp", "type": "uint256" },
            { "name": "owner", "type": "address" }
        ]
    })
}

fn address_verification_domain_with_chain(chain_id: u16) -> Value {
    json!({
      "name": "Farcaster Verify Ethereum Address",
      "version": "2.0.0",
      // fixed salt to minimize collisions
      "salt": "0xf2d857f4a3edcb9b78b4d503bfe733db1e3f6cdc2b7971ee739626c97e86a558",
      "chainId": chain_id,
    })
}

fn address_verification_domain() -> Value {
    json!({
      "name": "Farcaster Verify Ethereum Address",
      "version": "2.0.0",
      // fixed salt to minimize collisions
      "salt": "0xf2d857f4a3edcb9b78b4d503bfe733db1e3f6cdc2b7971ee739626c97e86a558",
    })
}

pub fn name_registry_domain() -> Value {
    json!({
        "name": "Farcaster name verification",
        "version": "1",
        "chainId": 1,
        "verifyingContract": "0xe3be01d99baa8db9905b33a3ca391238234b79d1" // name registry contract, will be the farcaster ENS CCIP contract later
    })
}

pub fn validate_fname_transfer(
    transfer: &proto::FnameTransfer,
    network: FarcasterNetwork,
    signer_address: Option<alloy_primitives::Address>,
) -> Result<(), ValidationError> {
    let proof = transfer.proof.as_ref().unwrap();
    let username = std::str::from_utf8(&proof.name);
    if username.is_err() {
        return Err(ValidationError::InvalidUsername);
    }

    let json = json!({
        "types": eip_712_domain(),
        "primaryType": "UserNameProof",
        "domain": name_registry_domain(),
        "message": {
            "name": username.unwrap(),
            "timestamp": proof.timestamp,
            "owner": hex::encode(proof.owner.clone())
        }
    });

    let typed_data = serde_json::from_value::<TypedData>(json);
    if typed_data.is_err() {
        return Err(ValidationError::InvalidData);
    }

    let data = typed_data.unwrap();
    let prehash = data.eip712_signing_hash();
    if prehash.is_err() {
        return Err(ValidationError::InvalidHash);
    }

    if network == FarcasterNetwork::Devnet {
        // Don't validate signatures on devnet (tests)
        return Ok(());
    }

    if proof.signature.len() != 65 {
        return Err(ValidationError::InvalidSignature);
    }

    let hash = prehash.unwrap();
    let fname_signer = signer_address.unwrap_or(FNAME_SIGNER_ADDRESS);
    let signature = alloy_primitives::PrimitiveSignature::from_bytes_and_parity(
        &proof.signature[0..64],
        proof.signature[64] != 0x1b && proof.signature[64] != 0x00,
    );

    let recovered_address = signature.recover_address_from_prehash(&hash);
    if recovered_address.is_err() {
        return Err(ValidationError::InvalidSignature);
    }

    let recovered = recovered_address.unwrap();
    if recovered != fname_signer {
        return Err(ValidationError::InvalidSignature);
    }

    Ok(())
}

pub fn validate_eth_address(address: &Vec<u8>) -> Result<&Vec<u8>, ValidationError> {
    if address.len() == 0 {
        return Err(ValidationError::EthAddressMissing);
    }

    if address.len() != 20 {
        return Err(ValidationError::InvalidEthAddressLength);
    }

    Ok(address)
}

fn validate_eth_block_hash(block_hash: &Vec<u8>) -> Result<&Vec<u8>, ValidationError> {
    if block_hash.len() == 0 {
        return Err(ValidationError::BlockHashMissing);
    }

    if block_hash.len() != 32 {
        return Err(ValidationError::InvalidBlockhashLength);
    }

    Ok(block_hash)
}

pub fn validate_sol_address(address: &Vec<u8>) -> Result<&Vec<u8>, ValidationError> {
    if address.len() == 0 {
        return Err(ValidationError::SolAddressMissing);
    }

    if address.len() != 32 {
        return Err(ValidationError::InvalidSolAddressLength);
    }

    Ok(address)
}

fn validate_sol_block_hash(block_hash: &Vec<u8>) -> Result<&Vec<u8>, ValidationError> {
    if block_hash.len() == 0 {
        return Err(ValidationError::BlockHashMissing);
    }

    if block_hash.len() != 32 {
        return Err(ValidationError::InvalidBlockhashLength);
    }

    Ok(block_hash)
}

fn validate_verification_eoa_signature(
    claim: VerificationAddressClaim,
    body: &VerificationAddAddressBody,
) -> Result<(), ValidationError> {
    let json = json!({
        "address": hex::encode(body.address.clone()),
        "types": eip_712_farcaster_verification_claim(),
        "primaryType": "VerificationClaim",
        "domain": address_verification_domain(),
        "message": {
          "fid": claim.fid,
          "address": claim.address,
          "blockHash": claim.block_hash,
          "network": claim.network,
        },
    });

    let typed_data = serde_json::from_value::<TypedData>(json);
    if typed_data.is_err() {
        return Err(ValidationError::InvalidData);
    }

    let data = typed_data.unwrap();
    let prehash = data.eip712_signing_hash();
    if prehash.is_err() {
        return Err(ValidationError::InvalidHash);
    }

    if body.claim_signature.len() != 65 {
        return Err(ValidationError::InvalidClaimSignature);
    }

    let hash = prehash.unwrap();
    let signature = alloy_primitives::PrimitiveSignature::from_bytes_and_parity(
        &body.claim_signature[0..64],
        body.claim_signature[64] != 0x1b && body.claim_signature[64] != 0x00,
    );

    let recovered_address = signature.recover_address_from_prehash(&hash);
    if recovered_address.is_err() {
        return Err(ValidationError::InvalidClaimSignature);
    }

    let recovered = recovered_address.unwrap().to_vec();
    if recovered != body.address {
        return Err(ValidationError::InvalidClaimSignature);
    }

    Ok(())
}

pub async fn validate_verification_contract_signature<P, T>(
    provider: P,
    claim: VerificationAddressClaim,
    body: &VerificationAddAddressBody,
) -> Result<(), ValidationError>
where
    P: Provider<T>,
    T: Transport + Clone,
{
    let json = json!({
        "types": eip_712_farcaster_verification_claim(),
        "primaryType": "VerificationClaim",
        "domain": address_verification_domain_with_chain(body.chain_id as u16),
        "message": {
          "fid": claim.fid,
          "address": claim.address,
          "blockHash": claim.block_hash,
          "network": claim.network,
        },
    });

    let typed_data = serde_json::from_value::<TypedData>(json);
    if typed_data.is_err() {
        return Err(ValidationError::InvalidData);
    }

    let data = typed_data.unwrap();
    let prehash = data.eip712_signing_hash();
    if prehash.is_err() {
        return Err(ValidationError::InvalidHash);
    }

    let hash = prehash.unwrap();

    match eth_signature_verifier::verify_signature(
        alloy_primitives::Bytes::from(body.claim_signature.clone()),
        alloy_primitives::Address::from(&body.address.clone().try_into().unwrap()),
        hash,
        &provider,
    )
    .await
    {
        Ok(verification) => match verification {
            Verification::Valid => Ok(()),
            Verification::Invalid => Err(ValidationError::InvalidClaimSignature),
        },
        Err(_) => Err(ValidationError::InvalidClaimSignature),
    }
}

#[derive(Debug, Serialize)]
pub struct VerificationAddressClaim {
    fid: u64,
    address: String,
    network: i32,
    block_hash: String,
    protocol: i32,
}

pub fn make_verification_address_claim(
    fid: u64,
    address: &Vec<u8>,
    network: proto::FarcasterNetwork,
    block_hash: &Vec<u8>,
    protocol: proto::Protocol,
) -> Result<VerificationAddressClaim, ValidationError> {
    match protocol {
        proto::Protocol::Ethereum => {
            let eth_address_hex = validate_eth_address(address);
            if eth_address_hex.is_err() {
                return Err(eth_address_hex.unwrap_err());
            }

            let block_hash_hex = validate_eth_block_hash(block_hash);
            if block_hash_hex.is_err() {
                return Err(block_hash_hex.unwrap_err());
            }

            Ok(VerificationAddressClaim {
                fid,
                address: hex::encode(address),
                network: network as i32,
                block_hash: hex::encode(block_hash),
                protocol: 0,
            })
        }
        proto::Protocol::Solana => {
            let sol_address = validate_sol_address(address);
            if sol_address.is_err() {
                return Err(sol_address.unwrap_err());
            }

            let block_hash_sol = validate_sol_block_hash(block_hash);
            if block_hash_sol.is_err() {
                return Err(block_hash_sol.unwrap_err());
            }

            Ok(VerificationAddressClaim {
                fid,
                address: bs58::encode(address).into_string(),
                network: network as i32,
                block_hash: bs58::encode(block_hash).into_string(),
                protocol: 1,
            })
        }
    }
}

fn validate_verification_add_eth_address_signature(
    body: &proto::VerificationAddAddressBody,
    fid: u64,
    network: proto::FarcasterNetwork,
) -> Result<(), ValidationError> {
    if body.claim_signature.len() > 2048 {
        return Err(ValidationError::InvalidEthClaimSignatureLength);
    }

    let chain_id = body.chain_id as u16;
    if !EIP_712_FARCASTER_VERIFICATION_CLAIM_CHAIN_IDS.contains(&chain_id) {
        return Err(ValidationError::InvalidData);
    }

    let reconstructed_claim = make_verification_address_claim(
        fid,
        &body.address,
        network,
        &body.block_hash,
        proto::Protocol::Ethereum,
    );

    if reconstructed_claim.is_err() {
        return Err(ValidationError::InvalidData);
    }

    match body.verification_type {
        0 => validate_verification_eoa_signature(reconstructed_claim.unwrap(), body),
        // Verification of contract signatures must happen out of consensus loop.
        1 => Ok(()),
        _ => Err(ValidationError::InvalidData),
    }
}

fn recreate_solana_claim_message(claim: VerificationAddressClaim) -> Vec<u8> {
    // We're using a simple ascii string instead of the full offchain signing spec because this provides better compatibility with wallet libraries
    let message_content = format!(
        "fid: {} address: {} network: {} blockHash: {} protocol: {}",
        claim.fid, claim.address, claim.network, claim.block_hash, claim.protocol
    );

    message_content.into_bytes()
}

fn validate_verification_add_sol_address_signature(
    body: &proto::VerificationAddAddressBody,
    fid: u64,
    network: proto::FarcasterNetwork,
) -> Result<(), ValidationError> {
    if body.claim_signature.len() != 64 {
        return Err(ValidationError::InvalidSolClaimSignatureLength);
    }

    let reconstructed_claim = make_verification_address_claim(
        fid,
        &body.address,
        network,
        &body.block_hash,
        proto::Protocol::Solana,
    );

    if reconstructed_claim.is_err() {
        return Err(ValidationError::InvalidData);
    }

    let full_message = recreate_solana_claim_message(reconstructed_claim.unwrap());

    let public_key =
        ed25519_dalek::VerifyingKey::from_bytes(body.address.as_slice().try_into().unwrap());

    if public_key.is_err() {
        return Err(ValidationError::InvalidData);
    }

    let signature =
        ed25519_dalek::Signature::from_bytes(body.claim_signature.as_slice().try_into().unwrap());

    match public_key.unwrap().verify_strict(&full_message, &signature) {
        Ok(_) => Ok(()),
        Err(_) => Err(ValidationError::InvalidSignature),
    }
}

fn validate_add_eth_address(
    body: &proto::VerificationAddAddressBody,
    fid: u64,
    network: proto::FarcasterNetwork,
) -> Result<(), ValidationError> {
    let valid_address = validate_eth_address(&body.address);
    if valid_address.is_err() {
        return Err(valid_address.unwrap_err());
    }

    let valid_block_hash = validate_eth_block_hash(&body.block_hash);
    if valid_block_hash.is_err() {
        return Err(valid_block_hash.unwrap_err());
    }

    let valid_signature = validate_verification_add_eth_address_signature(body, fid, network);
    if valid_signature.is_err() {
        return Err(valid_signature.unwrap_err());
    }

    Ok(())
}

fn validate_add_sol_address(
    body: &proto::VerificationAddAddressBody,
    fid: u64,
    network: proto::FarcasterNetwork,
) -> Result<(), ValidationError> {
    let valid_address = validate_sol_address(&body.address);
    if valid_address.is_err() {
        return Err(valid_address.unwrap_err());
    }

    let valid_block_hash = validate_sol_block_hash(&body.block_hash);
    if valid_block_hash.is_err() {
        return Err(valid_block_hash.unwrap_err());
    }

    let valid_signature = validate_verification_add_sol_address_signature(body, fid, network);
    if valid_signature.is_err() {
        return Err(valid_signature.unwrap_err());
    }

    Ok(())
}

pub fn validate_add_address(
    body: &proto::VerificationAddAddressBody,
    fid: u64,
    network: proto::FarcasterNetwork,
) -> Result<(), ValidationError> {
    match body.protocol {
        x if x == proto::Protocol::Ethereum as i32 => validate_add_eth_address(body, fid, network),
        x if x == proto::Protocol::Solana as i32 => validate_add_sol_address(body, fid, network),
        _ => Err(ValidationError::InvalidData),
    }
}

fn validate_remove_eth_address(
    body: &proto::VerificationRemoveBody,
) -> Result<(), ValidationError> {
    let valid_address = validate_eth_address(&body.address);
    if valid_address.is_err() {
        return Err(valid_address.unwrap_err());
    }

    Ok(())
}

fn validate_remove_sol_address(
    body: &proto::VerificationRemoveBody,
) -> Result<(), ValidationError> {
    let valid_address = validate_sol_address(&body.address);
    if valid_address.is_err() {
        return Err(valid_address.unwrap_err());
    }

    Ok(())
}

pub fn validate_remove_address(
    body: &proto::VerificationRemoveBody,
) -> Result<(), ValidationError> {
    match body.protocol {
        x if x == proto::Protocol::Ethereum as i32 => validate_remove_eth_address(body),
        x if x == proto::Protocol::Solana as i32 => validate_remove_sol_address(body),
        _ => Err(ValidationError::InvalidData),
    }
}
