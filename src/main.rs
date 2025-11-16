use alloy::{
    dyn_abi::DynSolValue, network::TransactionBuilder, node_bindings::Anvil, primitives::{address, keccak256, Address, FixedBytes, U256}, providers::{Provider, ProviderBuilder}, rpc::types::TransactionRequest, signers::{local::PrivateKeySigner, Signer}, sol, sol_types::{eip712_domain, SolEvent, SolStruct, SolValue}
};

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    Example,
    "contracts/Example.json"
);

fn hash_domain() -> FixedBytes<32> {
    let domain_type = keccak256(
        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let mut encoded = Vec::new();
    encoded.extend_from_slice(domain_type.as_slice());
    encoded.extend_from_slice(keccak256("Ether Mail").as_slice());
    encoded.extend_from_slice(keccak256("1").as_slice());
    encoded.extend_from_slice(&DynSolValue::Uint(U256::from(1), 256).abi_encode());
    encoded.extend_from_slice(&address!("0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC").abi_encode());

    keccak256(&encoded).into()
}

fn hash_person(name: &str, wallet: Address) -> FixedBytes<32> {
    let person_typehash = keccak256(b"Person(string name,address wallet)").to_vec();

    let mut encoded = Vec::new();
    encoded.extend_from_slice(&person_typehash);
    encoded.extend_from_slice(&keccak256(name.as_bytes()).to_vec());
    encoded.extend_from_slice(&wallet.abi_encode());

    keccak256(&encoded).into()
}

fn hash_mail(from: (&str, Address), to: (&str, Address), contents: &str) -> FixedBytes<32> {
    let mail_typehash =
        keccak256(b"Mail(Person from,Person to,string contents)Person(string name,address wallet)");

    let mut encoded = Vec::new();
    encoded.extend_from_slice(mail_typehash.as_slice());
    encoded.extend_from_slice(hash_person(from.0, from.1).as_slice());
    encoded.extend_from_slice(hash_person(to.0, to.1).as_slice());
    encoded.extend_from_slice(keccak256(contents.as_bytes()).as_slice());

    keccak256(&encoded).into()
}

#[tokio::main]
async fn main() {
    let anvil = Anvil::new().block_time(1).try_spawn().unwrap();

    let signer_alice: PrivateKeySigner = anvil.keys()[0].clone().into();
    let signer_bob: PrivateKeySigner = anvil.keys()[1].clone().into();

    let alice = signer_alice.address();
    let bob = signer_bob.address();

    let domain_separator = hash_domain();
    let mail_hash = hash_mail(("Alice", alice), ("Bob", bob), "Hello, Bob!");
    let eip712_hash = keccak256([&[0x19, 0x01], &domain_separator[..], &mail_hash[..]].concat());

    let sign = signer_alice.sign_hash(&eip712_hash).await.unwrap();
    let recovered_address = sign.recover_address_from_prehash(&eip712_hash).unwrap();
    assert_eq!(recovered_address, alice);

    let mail = {
        let mail = Example::Mail {
            from: Example::Person {
                name: "Alice".to_string(),
                wallet: alice,
            },
            to: Example::Person {
                name: "Bob".to_string(),
                wallet: bob,
            },
            contents: "Hello, Bob!".to_string(),
        };

        let domain = eip712_domain! {
            name: "Ether Mail",
            version: "1",
            chain_id: 1,
            verifying_contract: address!("0xCcCCccccCCCCcCCCCCCcCcCccCcCCCcCcccccccC"),
        };

        let sign_1 = signer_alice.sign_typed_data(&mail, &domain).await.unwrap();
        assert_eq!(sign_1, sign);
        assert_eq!(domain.hash_struct(), domain_separator);
        assert_eq!(mail.eip712_hash_struct(), mail_hash);

        mail
    };

    let rpc_url = anvil.endpoint_url();
    let provider = ProviderBuilder::new()
        .wallet(signer_alice)
        .connect_http(rpc_url.clone());
    let contract = Example::deploy(&provider).await.unwrap();


    let r = FixedBytes::<32>::from(sign.r());
    let s = FixedBytes::<32>::from(sign.s());
    let v = if sign.v() { 28 } else { 27 };
    let result = contract.verify(mail.clone(), v, r, s).call().await.unwrap();
    assert!(result);

    // Bob pay for gas
    let provider = ProviderBuilder::new()
        .wallet(signer_bob)
        .connect_http(rpc_url);
    let data =  contract.verifyAndEmit(mail, v, r, s).calldata().clone();
    let tx = TransactionRequest::default()
    .with_to(*contract.address())
    .with_input(data)
    .with_chain_id(provider.get_chain_id().await.unwrap())
    .with_gas_limit(100_000)
    .with_max_priority_fee_per_gas(1_000_000_000)
    .with_max_fee_per_gas(20_000_000_000);

    let pending_tx = provider.send_transaction(tx).await.unwrap(); 
    let receipt = pending_tx.get_receipt().await.unwrap();

    for log in receipt.logs() {
        if let Ok(ev) = Example::VerifyStatus::decode_log(&log.inner) {
            println!("VerifyStatus event: status = {}", ev.status);
        }
    }
    
}
