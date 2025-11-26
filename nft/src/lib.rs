/*!
Non-Fungible Token implementation with JSON serialization.
NOTES:
  - The maximum balance value is limited by U128 (2**128 - 1).
  - JSON calls should pass U128 as a base-10 string. E.g. "100".
  - The contract optimizes the inner trie structure by hashing account IDs. It will prevent some
    abuse of deep tries. Shouldn't be an issue, once NEAR clients implement full hashing of keys.
  - The contract tracks the change in storage before and after the call. If the storage increases,
    the contract requires the caller of the contract to attach enough deposit to the function call
    to cover the storage cost.
    This is done to prevent a denial of service attack on the contract by taking all available storage.
    If the storage decreases, the contract will issue a refund for the cost of the released storage.
    The unused tokens from the attached deposit are also refunded, so it's safe to
    attach more deposit than required.
  - To prevent the deployed contract from being modified or deleted, it should not have any access
    keys on its account.
*/
use near_contract_standards::non_fungible_token::approval::NonFungibleTokenApproval;
use near_contract_standards::non_fungible_token::core::{
    NonFungibleTokenCore, NonFungibleTokenResolver,
};
use near_contract_standards::non_fungible_token::enumeration::NonFungibleTokenEnumeration;
use near_contract_standards::non_fungible_token::metadata::{
    NFTContractMetadata, NonFungibleTokenMetadataProvider, TokenMetadata,
};
use near_contract_standards::non_fungible_token::NonFungibleToken;
use near_contract_standards::non_fungible_token::events::NftMint;
use near_contract_standards::non_fungible_token::{Token, TokenId};
use near_contract_standards::fungible_token::{Balance};
use near_sdk::{assert_one_yocto, is_promise_success};
use near_sdk::serde::{Serialize, Deserialize};
use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::collections::{LazyOption, LookupMap, UnorderedSet};
use near_sdk::json_types::U128;
use near_sdk::{near, 
    env, near_bindgen, require, AccountId, BorshStorageKey, PanicOnDefault, Promise, PromiseOrValue, NearToken, Gas, 
    serde_json::json,
};
use rs_merkle::{ MerkleProof, Hasher };
use rs_merkle::algorithms::Sha256;
use std::collections::HashMap;

mod ft_balances;

#[near(serializers=[json])] 
pub struct Payout {
    pub payout: HashMap<AccountId, U128>,
}


#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
#[borsh(crate = "near_sdk::borsh")]
pub struct Contract {
    pub tokens: NonFungibleToken,

    pub metadata: LazyOption<NFTContractMetadata>,

    pub index: u128,

    pub total_supply: u128,

    pub mint_price: u128,

    pub wl_price: u128,
    
    //which fungible token can be used to purchase NFTs
    pub mint_currency: Option<AccountId>, 
    
    pub payment_split_percent: u128,

    //keep track of how many FTs each account has deposited in order to purchase NFTs with
    pub ft_deposits: LookupMap<AccountId, Balance>,

    pub nft_price: LookupMap<TokenId, u128>,

    pub burn_fee: u128,

    pub balances_by_owner: LookupMap<AccountId, Balance>,

    pub holders: UnorderedSet<AccountId>,

    pub minters: UnorderedSet<AccountId>,

    pub treasury: AccountId,

    pub royalty: u128,

    pub root_hash: String,

    pub is_public_mint: bool, 

    pub is_unique_mint: bool,

    pub whitelist_count: u32
}

const NEAR_PER_STORAGE: u128 = 10_000_000_000_000_000_000;
//the minimum storage to have a sale on the contract.
const VAULT_STORAGE: u128 = 19_800_000_000_000_000_000_000;

#[derive(BorshSerialize, BorshStorageKey)]
#[borsh(crate = "near_sdk::borsh")]
enum StorageKey {
    NonFungibleToken,
    Metadata,
    TokenMetadata,
    Enumeration,
    Approval,
    FTDeposits,
    BalancesByOwner,
    Holders,
    Minters,
    NftPrice
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new(
        owner_id: AccountId,
        metadata: NFTContractMetadata,
        mint_price: U128,
        wl_price: U128,
        mint_currency: Option<AccountId>,
        payment_split_percent: U128,
        total_supply: U128,
        burn_fee: U128,
        treasury: AccountId,
        royalty: U128,
        root_hash: String,
        whitelist_count: u32
    ) -> Self {
        require!(!env::state_exists(), "Already initialized");
        metadata.assert_valid();
        Self {
            tokens: NonFungibleToken::new(
                StorageKey::NonFungibleToken,
                owner_id,
                Some(StorageKey::TokenMetadata),
                Some(StorageKey::Enumeration),
                Some(StorageKey::Approval),
            ),
            metadata: LazyOption::new(StorageKey::Metadata, Some(&metadata)),
            index: 0,
            total_supply: total_supply.0,
            mint_price: mint_price.0,
            wl_price: wl_price.0,
            mint_currency,
            payment_split_percent: payment_split_percent.0,
            ft_deposits: LookupMap::new(StorageKey::FTDeposits),
            burn_fee: burn_fee.0,
            balances_by_owner: LookupMap::new(StorageKey::BalancesByOwner),
            nft_price: LookupMap::new(StorageKey::NftPrice),
            holders: UnorderedSet::new(StorageKey::Holders),
            minters: UnorderedSet::new(StorageKey::Minters),
            treasury: treasury,
            royalty: royalty.0,
            root_hash,
            is_public_mint: false,
            whitelist_count: whitelist_count,
            is_unique_mint: true
        }
    }

    #[payable]
    pub fn set_mint_type(&mut self, is_public_mint: bool, is_unique_mint: bool) {
        assert_one_yocto();
        let owner = env::predecessor_account_id();
        require!(owner == self.tokens.owner_id, "DS: You are not an owner.");
        self.is_public_mint = is_public_mint;
        self.is_unique_mint = is_unique_mint;
    }

    #[payable]
    pub fn set_root_hash(&mut self, root_hash: String, whitelist_count: u32) {
        assert_one_yocto();
        let owner = env::predecessor_account_id();
        require!(owner == self.tokens.owner_id, "DS: You are not an owner.");
        self.whitelist_count = whitelist_count;
        self.root_hash = root_hash;
    }
    /// Mint a new token with ID=`token_id` belonging to `token_owner_id`.
    ///
    /// Since this example implements metadata, it also requires per-token metadata to be provided
    /// in this call. `self.tokens.mint` will also require it to be Some, since
    /// `StorageKey::TokenMetadata` was provided at initialization.
    ///
    /// `self.tokens.mint` will enforce `predecessor_account_id` to equal the `owner_id` given in
    /// initialization call to `new`.
    #[payable]
    pub fn nft_mint(
        &mut self,
        token_owner_id: AccountId,
        token_metadata: TokenMetadata,
        proof: String,
        leaf_index: u32
    ) -> Token {
        let mut price = self.mint_price;
        if !self.is_public_mint {
            price = self.wl_price;
            let proof_bytes: Vec<u8> = hex::decode(proof).expect("DS: Invalid proof");
            // Parse proof back on the client
            let proof = MerkleProof::<Sha256>::try_from(proof_bytes.clone()).unwrap();
            let merkle_root_vec = hex::decode(&self.root_hash).expect("DS: Invalid proof");
            let merkle_root: [u8; 32] = merkle_root_vec.try_into().map_err(|_| "Invalid merkle root").unwrap();
            let leaves_to_prove = [Sha256::hash(env::predecessor_account_id().as_bytes())];
            require!(proof.verify(merkle_root, &[leaf_index as usize], &leaves_to_prove, self.whitelist_count as usize), "DS: This user isn't whitelisted.");
        }

        let collection_owner = &self.tokens.owner_id;
        let owner = env::predecessor_account_id();
        if self.is_unique_mint {
            require!(!self.minters.contains(&owner), "DS: You already minted one.")
        }
        let token_id:TokenId = (self.index + 1).to_string();
        self.holders.insert(&owner);
        self.minters.insert(&owner);
        let code = include_bytes!("./vault/vault.wasm").to_vec();
        let contract_bytes = code.len() as u128;
        let minimum_needed = NEAR_PER_STORAGE * contract_bytes + VAULT_STORAGE;
        let deposit: u128 = env::attached_deposit().as_yoctonear();
        if let Some(_) = self.mint_currency.clone() {
            let mut amount = self.ft_deposits_of(owner.clone());
            require!(deposit >= minimum_needed && amount >= price, "Insufficient price to mint");
            amount -= price;
            self.ft_deposits.insert(&owner, &amount);
        } else {
            require!(deposit >= price + minimum_needed, "Insufficient price to mint");
        }

        let current_id = env::current_account_id();

        let vault_amount = price.checked_mul(self.payment_split_percent)
            .unwrap().checked_div(100u128).unwrap();

        let owner_amount = price.checked_sub(vault_amount).unwrap();

        // Deploy the vault contract
        let vault_account_id: AccountId = format!("{}.{}", token_id, current_id).parse().unwrap();
        Promise::new(vault_account_id.clone())
            .create_account()
            .deploy_contract(code)
            .transfer(NearToken::from_yoctonear(deposit))
            .function_call(
                // Init the vault contract
                "init".to_string(),
                if let Some(ft_id) = self.mint_currency.clone() {
                    json!({
                        "ft_contract": ft_id.to_string(),
                        "treasury": self.treasury.to_string(),
                        "admin": self.tokens.owner_id.to_string()
                    })
                } else {
                    json!({
                        "treasury": self.treasury.to_string(),
                        "admin": self.tokens.owner_id.to_string()
                    })
                }.to_string().into_bytes().to_vec(),
                NearToken::from_millinear(0),
                Gas::from_tgas(50)
            )
            .then(
                Self::ext(env::current_account_id())
                .with_static_gas(Gas::from_tgas(150))
                .resolve_create(
                    vault_account_id,
                    collection_owner,
                    owner_amount,
                    vault_amount
                )
            );
        self.index = self.index.checked_add(1).unwrap();
        if self.total_supply > 0 {
            require!(self.total_supply >= self.index, "Exceeded total supply");
        }

        let token = self.tokens.internal_mint_with_refund(token_id.clone(), token_owner_id, Some(token_metadata), None);
        self.nft_price.insert(&token_id, &price);
        NftMint { owner_id: &token.owner_id, token_ids: &[&token.token_id], memo: None }.emit();
        token
    }
    #[private]
    pub fn resolve_create(
        &mut self,
        vault_account_id:AccountId,
        collection_owner:&AccountId,
        owner_amount: u128,
        vault_amount: u128
    ) -> Promise {
        assert!(is_promise_success(), "DS: Vault creation failed");
        // Deposit ft or near
        if let Some(ft_id) = self.mint_currency.clone() {
            Promise::new(ft_id.clone()).function_call(
                "ft_transfer_call".to_string(), 
                json!({
                    "receiver_id": vault_account_id.to_string(),
                    "amount": vault_amount.to_string(),
                    "msg": "",
                }).to_string().into_bytes().to_vec(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(50),
            );
            Promise::new(ft_id.clone()).function_call(
                "ft_transfer".to_string(), 
                json!({
                    "receiver_id": collection_owner.clone().to_string(),
                    "amount": owner_amount.to_string(),
                    "msg": "",
                }).to_string().into_bytes().to_vec(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(50),
            )
        } else {
            Promise::new(collection_owner.clone()).transfer(NearToken::from_yoctonear(owner_amount));
            Promise::new(vault_account_id.clone()).function_call(
                "deposit_near".to_string(),
                json!({}).to_string().into_bytes().to_vec(),
                NearToken::from_yoctonear(vault_amount),
                Gas::from_tgas(20),
            )
        }
    }
    // Burn an NFT by its token ID
    #[payable]
    pub fn burn(&mut self, token_id: TokenId) {
        assert_one_yocto();
        let owner = env::predecessor_account_id();

        let token_owner = self.tokens.owner_by_id.get(&token_id).unwrap();
        let mint_price = self.nft_price.get(&token_id).unwrap();
        require!(owner.clone() == token_owner, "You don't own this NFT");

        // Remove the NFT from the owner's account
        self.tokens.owner_by_id.remove(&token_id);

        // Remove token metadata (if applicable)
        self.tokens
            .token_metadata_by_id
            .as_mut()
            .and_then(|by_id| by_id.remove(&token_id));
        
        // Remove the NFT from the tokens_per_owner map
        let mut removed = false;
        if let Some(tokens_per_owner) = &mut self.tokens.tokens_per_owner {
            let mut owner_tokens = tokens_per_owner.get(&owner).unwrap_or_else(|| {
                env::panic_str("Unable to access tokens per owner in unguarded call.")
            });
            owner_tokens.remove(&token_id);
            if owner_tokens.is_empty() {
                tokens_per_owner.remove(&owner);
                self.holders.remove(&owner);
                removed = true;
            } else {
                tokens_per_owner.insert(&owner, &owner_tokens);
            }
        }
        
        // Remove any approvals associated with this NFT
        self.tokens
            .approvals_by_id
            .as_mut()
            .and_then(|by_id| by_id.remove(&token_id.clone()));

        // Remove next approval ID (if applicable)
        self.tokens
            .next_approval_id_by_id
            .as_mut()
            .and_then(|by_id| by_id.remove(&token_id.clone()));

        // Update Balance for holders
        let mut holders_count: u128 = self.holders.len() as u128;
        if removed == false {
            holders_count -= 1;
        }
        let amount_to_holder: u128 = if holders_count == 0 {
            0u128
        } else { 
            mint_price
                .checked_mul(self.payment_split_percent).unwrap()
                .checked_mul(self.burn_fee).unwrap()
                .checked_div(20000u128).unwrap()
                .checked_div(holders_count).unwrap()
        };

        env::log_str(&format!("Total holders count: {}", holders_count));
        env::log_str(&format!("Amount to each holder: {}", amount_to_holder));

        for other in self.holders.iter() {
            if other != owner {
                let mut balance = self.balances_by_owner.get(&other).unwrap_or(0);
                balance = balance.checked_add(amount_to_holder).unwrap();
                self.balances_by_owner.insert(&other, &balance);
            }
        }

        let current_id = env::current_account_id();
        let vault_account_id: AccountId = format!("{}.{}", token_id, current_id).parse().unwrap();
        Promise::new(vault_account_id.clone()).function_call(
            "withdraw".to_string(),
            json!({
                "owner": owner.to_string(),
                "burn_fee": self.burn_fee.to_string(),
            }).to_string().into_bytes().to_vec(),
            NearToken::from_yoctonear(1),
            Gas::from_tgas(200)
        );
    }

    #[payable]
    pub fn withdraw(&mut self) {
        assert_one_yocto();
        let owner = env::predecessor_account_id();
        let balance: u128 = self.balances_by_owner.get(&owner).unwrap_or(0);

        if balance > 0 {
            // Deposit ft or near
            if let Some(ft_id) = self.mint_currency.clone() {
                Promise::new(ft_id.clone()).function_call(
                    "ft_transfer".to_string(), 
                    json!({
                        "receiver_id": owner.to_string(),
                        "amount": balance.to_string(),
                    }).to_string().into_bytes().to_vec(),
                    NearToken::from_yoctonear(1),
                    Gas::from_tgas(20),
                ).then(
                    Self::ext(env::current_account_id())
                        .with_static_gas(Gas::from_tgas(10))
                        .callback_withdraw(owner.clone(), balance)
                );
            } else {
                Promise::new(owner.clone())
                    .transfer(NearToken::from_yoctonear(balance))
                    .then(
                        Self::ext(env::current_account_id())
                            .with_static_gas(Gas::from_tgas(10))
                            .callback_withdraw(owner.clone(), balance)
                    );
            }

            self.balances_by_owner.insert(&owner, &0u128).unwrap();
        }
    }
    #[payable]
    pub fn withdraw_tokens(&mut self) {
        assert_one_yocto();
        let owner = env::predecessor_account_id();
        let balance = self.ft_deposits.get(&owner).unwrap_or(0);
        if balance > 0 {
            if let Some(ft_id) = self.mint_currency.clone() {
                Promise::new(ft_id.clone()).function_call(
                    "ft_transfer".to_string(), 
                    json!({
                        "receiver_id": owner.to_string(),
                        "amount": balance.to_string(),
                    }).to_string().into_bytes().to_vec(),
                    NearToken::from_yoctonear(1),
                    Gas::from_tgas(20),
                ).then(
                    Self::ext(env::current_account_id())
                        .with_static_gas(Gas::from_tgas(10))
                        .callback_withdraw_tokens(owner.clone(), balance)
                );
                self.ft_deposits.insert(&owner, &0u128);
            }
        }
    }

    #[private] 
    pub fn callback_withdraw( 
        &mut self, 
        owner: AccountId, 
        amount: u128, 
    ) { 
        if !is_promise_success() {
            self.balances_by_owner.insert(&owner, &amount).unwrap();
        };
    }
    #[private] 
    pub fn callback_withdraw_tokens( 
        &mut self, 
        owner: AccountId, 
        amount: u128, 
    ) { 
        if !is_promise_success() {
            self.ft_deposits.insert(&owner, &amount).unwrap();
        };
    }

    #[payable]
    pub fn nft_transfer_payout(
        &mut self,
        receiver_id: AccountId,
        token_id: TokenId,
        approval_id: Option<u64>,
        balance: Option<U128>
    ) -> Option<Payout> {
        assert_one_yocto();
        let previous_owner_id =
            self.tokens.owner_by_id.get(&token_id).unwrap_or_else(|| env::panic_str("Token not found"));
        if let Some(tokens_per_owner) = &mut self.tokens.tokens_per_owner {
            let sender_tokens = tokens_per_owner.get(&previous_owner_id).unwrap_or_else(|| {
                env::panic_str("Unable to access tokens per owner in unguarded call.")
            });
            if sender_tokens.len()==1 {
                self.holders.remove(&previous_owner_id);
            };
            let receiver_tokens = tokens_per_owner.get(&receiver_id);
            if receiver_tokens.is_none() {
                self.holders.insert(&receiver_id);
            } else {
                let receiver_tokens = receiver_tokens.unwrap();
                if receiver_tokens.len() == 0 {
                    self.holders.insert(&receiver_id);
                }
            }
        }
        self.tokens.nft_transfer(receiver_id, token_id, approval_id, None);

        let payout = if let Some(balance) = balance {
            let balance_u128: u128 = u128::from(balance);
            let mut payout: Payout = Payout {
                payout: HashMap::new(),
            };
            payout.payout.insert(self.tokens.owner_id.clone(), royalty_to_payout(self.royalty, balance_u128));
            payout.payout.insert(previous_owner_id, royalty_to_payout(10000-self.royalty, balance_u128));
            Some(payout)
        } else {
            None
        };
        payout
    }
    /// Get the amount of FTs the user has deposited into the contract
    pub fn ft_deposits_of(
        &self,
        account_id: AccountId
    ) -> u128 {
        self.ft_deposits.get(&account_id).unwrap_or(0)
    }

    pub fn index(&self) -> u128 {
        self.index
    }

    pub fn get_merkle_root(&self) -> String {
        self.root_hash.clone()
    }

    pub fn total_supply(&self) -> u128 {
        self.total_supply
    }

    pub fn balance_of(&self, owner: AccountId) -> u128 {
        self.balances_by_owner.get(&owner).unwrap_or(0)
    }

    pub fn total_holders(&self) -> u64 {
        self.holders.len()
    }

    pub fn is_mintable(&self, account: AccountId) -> bool {
        self.minters.contains(&account)
    }
}

#[near_bindgen]
impl NonFungibleTokenCore for Contract {
    #[payable]
    fn nft_transfer(
        &mut self,
        receiver_id: AccountId,
        token_id: TokenId,
        approval_id: Option<u64>,
        memo: Option<String>,
    ) {
        let owner_id =
            self.tokens.owner_by_id.get(&token_id).unwrap_or_else(|| env::panic_str("Token not found"));
        if let Some(tokens_per_owner) = &mut self.tokens.tokens_per_owner {
            let sender_tokens = tokens_per_owner.get(&owner_id).unwrap_or_else(|| {
                env::panic_str("Unable to access tokens per owner in unguarded call.")
            });
            if sender_tokens.len()==1 {
                self.holders.remove(&owner_id);
            };
            let receiver_tokens = tokens_per_owner.get(&receiver_id);
            if receiver_tokens.is_none() {
                self.holders.insert(&receiver_id);
            } else {
                let receiver_tokens = receiver_tokens.unwrap();
                if receiver_tokens.len() == 0 {
                    self.holders.insert(&receiver_id);
                }
            }
        }
        self.tokens.nft_transfer(receiver_id, token_id, approval_id, memo);
    }

    #[payable]
    fn nft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        token_id: TokenId,
        approval_id: Option<u64>,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<bool> {
        let owner_id =
            self.tokens.owner_by_id.get(&token_id).unwrap_or_else(|| env::panic_str("Token not found"));
        if let Some(tokens_per_owner) = &mut self.tokens.tokens_per_owner {
            let sender_tokens = tokens_per_owner.get(&owner_id).unwrap_or_else(|| {
                env::panic_str("Unable to access tokens per owner in unguarded call.")
            });
            if sender_tokens.len()==1 {
                self.holders.remove(&owner_id);
            };
            let receiver_tokens = tokens_per_owner.get(&receiver_id);
            if receiver_tokens.is_none() {
                self.holders.insert(&receiver_id);
            } else {
                let receiver_tokens = receiver_tokens.unwrap();
                if receiver_tokens.len() == 0 {
                    self.holders.insert(&receiver_id);
                }
            }
        }
        self.tokens.nft_transfer_call(receiver_id, token_id, approval_id, memo, msg)
    }

    fn nft_token(&self, token_id: TokenId) -> Option<Token> {
        self.tokens.nft_token(token_id)
    }
}

fn royalty_to_payout(a: u128, b: Balance) -> U128 {
    U128(a as u128 * b / 10_000u128)
}

#[near_bindgen]
impl NonFungibleTokenResolver for Contract {
    #[private]
    fn nft_resolve_transfer(
        &mut self,
        previous_owner_id: AccountId,
        receiver_id: AccountId,
        token_id: TokenId,
        approved_account_ids: Option<HashMap<AccountId, u64>>,
    ) -> bool {
        self.tokens.nft_resolve_transfer(
            previous_owner_id,
            receiver_id,
            token_id,
            approved_account_ids,
        )
    }
}

#[near_bindgen]
impl NonFungibleTokenApproval for Contract {
    #[payable]
    fn nft_approve(
        &mut self,
        token_id: TokenId,
        account_id: AccountId,
        msg: Option<String>,
    ) -> Option<Promise> {
        self.tokens.nft_approve(token_id, account_id, msg)
    }

    #[payable]
    fn nft_revoke(&mut self, token_id: TokenId, account_id: AccountId) {
        self.tokens.nft_revoke(token_id, account_id);
    }

    #[payable]
    fn nft_revoke_all(&mut self, token_id: TokenId) {
        self.tokens.nft_revoke_all(token_id);
    }

    fn nft_is_approved(
        &self,
        token_id: TokenId,
        approved_account_id: AccountId,
        approval_id: Option<u64>,
    ) -> bool {
        self.tokens.nft_is_approved(token_id, approved_account_id, approval_id)
    }
}

#[near_bindgen]
impl NonFungibleTokenEnumeration for Contract {
    fn nft_total_supply(&self) -> U128 {
        self.tokens.nft_total_supply()
    }

    fn nft_tokens(&self, from_index: Option<U128>, limit: Option<u64>) -> Vec<Token> {
        self.tokens.nft_tokens(from_index, limit)
    }

    fn nft_supply_for_owner(&self, account_id: AccountId) -> U128 {
        self.tokens.nft_supply_for_owner(account_id)
    }

    fn nft_tokens_for_owner(
        &self,
        account_id: AccountId,
        from_index: Option<U128>,
        limit: Option<u64>,
    ) -> Vec<Token> {
        self.tokens.nft_tokens_for_owner(account_id, from_index, limit)
    }
}

#[near_bindgen]
impl NonFungibleTokenMetadataProvider for Contract {
    fn nft_metadata(&self) -> NFTContractMetadata {
        self.metadata.get().unwrap()
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use rs_merkle::{MerkleProof, MerkleTree};
    use rs_merkle::algorithms::Sha256;
    use rs_merkle::Hasher;
    use super::*;
    use near_sdk::test_utils::{accounts, VMContextBuilder};
    use near_sdk::testing_env;
    use near_sdk::MockedBlockchain;
    fn get_context(predecessor_account_id: AccountId) -> VMContextBuilder {
        let mut builder = VMContextBuilder::new();
        builder
            .current_account_id(accounts(0))
            .signer_account_id(predecessor_account_id.clone())
            .predecessor_account_id(predecessor_account_id);
        builder
    }
    fn setup_contract(
        hash: String,
        len: u32
    ) -> (VMContextBuilder, Contract) {
        let mut context = VMContextBuilder::new();
        testing_env!(context.predecessor_account_id(accounts(0)).build());
        let metadata = NFTContractMetadata {
            spec: String::from("nft-1.0.0"),
            name: String::from("DS"),
            symbol: String::from("USS"),
            icon: None,
            base_uri: None,
            reference: None,
            reference_hash: None
        };
        let contract = Contract::new(
            accounts(0),
            metadata,
            U128::from(10000000),
            U128::from(10000000),
            None,
            U128::from(50),
            U128::from(4444),
            U128::from(10),
            accounts(1),
            U128::from(1000),
            hash,
            len
        );
        (context, contract)
        
    }


    #[test]
    fn test_merkle() {
        let leaf_values = [
            "defishards.near", 
            "devbose.near", 
            "mf-69.near", 
            "purre.near", 
            "lok07.near", 
            "olascious.near",
            "nelson1906.near",
            "liightrevival.near",
            "deirazabalqueen.near"
        ];
        let leaves: Vec<[u8; 32]> = leaf_values
            .iter()
            .map(|x| Sha256::hash(x.as_bytes()))
            .collect();
        let merkle_tree = MerkleTree::<Sha256>::from_leaves(&leaves);
        let indices_to_prove = vec![1];
        let leaves_to_prove = leaves.get(1..2).ok_or("can't get leaves to prove").unwrap();
        let merkle_proof = merkle_tree.proof(&indices_to_prove);
        let merkle_root = merkle_tree.root().ok_or("couldn't get the merkle root").unwrap();
        // Serialize proof to pass it to the client
        let proof_bytes: Vec<u8> = merkle_proof.to_bytes();
        // Parse proof back on the client
        let proof = MerkleProof::<Sha256>::try_from(proof_bytes.clone()).unwrap();
        let hex_root = hex::encode(merkle_root.clone());
        println!("mercle root {:?}", merkle_root);
        println!("hex root {:?}", hex_root);
        println!("{:?}", hex::decode(hex_root).unwrap());
        assert!(proof.verify(merkle_root, &indices_to_prove, leaves_to_prove, leaves.len()));
    }

    #[test]
    fn test_mint() {
        let leaf_values = [
            "defishards.near", 
            "devbose.near", 
            "mf-69.near", 
            "purre.near", 
            "lok07.near", 
            "olascious.near",
            "nelson1906.near",
            "liightrevival.near",
            "deirazabalqueen.near"
        ];
        println!("{:?}", leaf_values);
        let leaves: Vec<[u8; 32]> = leaf_values
            .iter()
            .map(|x| Sha256::hash(x.as_bytes()))
            .collect();
        let merkle_tree = MerkleTree::<Sha256>::from_leaves(&leaves);
        let merkle_root = merkle_tree.root().ok_or("couldn't get the merkle root").unwrap();
        let merkle_root_hash = hex::encode(merkle_root);
        println!("{:?}", merkle_root_hash);
        let (mut context, mut contract) = setup_contract(merkle_root_hash, leaf_values.len() as u32);
        testing_env!(context
            .predecessor_account_id(accounts(0))
            .attached_deposit(NearToken::from_near(10))
            .build());
        let token_metadata = TokenMetadata {
            title: Some("Tsundere land".to_string()),
            description: None,
            media: Some("newmedia".to_string()),
            media_hash: None,
            copies: None,
            issued_at: None,
            expires_at: None,
            starts_at: None,
            updated_at: None,
            extra: None,
            reference: Some("newreference".to_string()),
            reference_hash: None,
        };
        let indices_to_prove = vec![2];
        let merkle_proof = merkle_tree.proof(&indices_to_prove);
        let proof_bytes: Vec<u8> = merkle_proof.to_bytes();
        let proof_hash = hex::encode(proof_bytes);
        println!("Proof Hash: {:?}", proof_hash);
        // contract.set_mint_type(false);
        contract.nft_mint(accounts(0), token_metadata, proof_hash, indices_to_prove[0] as u32);
    }
}