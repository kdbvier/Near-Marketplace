use near_sdk::{assert_one_yocto, require};
// Find all our documentation at https://docs.near.org
use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::{
    near_bindgen, AccountId, env, Promise, NearToken, Gas,
    serde_json::json, log
};
use near_sdk::json_types::U128;
use near_contract_standards::non_fungible_token::metadata::NFTContractMetadata;
use near_sdk::serde::{ Serialize, Deserialize };

const NFT_CONTRACT_STORAGE: u128 = 30_000_000_000_000_000_000_000;

// Define the contract structure
#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize)]
#[borsh(crate = "near_sdk::borsh")]
pub struct Contract {
    treasury: AccountId,
    admin: AccountId,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct ConfigInfo {
    pub treasury: AccountId,
    pub admin: AccountId
}
impl Default for Contract {
    fn default() -> Self {
        Self {
            treasury: env::predecessor_account_id(),
            admin: env::predecessor_account_id()
        }
    }
}

// Implement the contract structure
#[near_bindgen]
impl Contract {
    #[init]
    pub fn init(treasury: AccountId, admin: AccountId) -> Self {
        Self {
            treasury: treasury,
            admin: admin
        }
    }

    #[payable]
    pub fn set_config(
        &mut self,
        treasury: AccountId,
        admin: AccountId
    ) {
        assert_one_yocto();
        assert_eq!(
            env::predecessor_account_id(),
            self.admin,
            "Admin only"
        );
        self.treasury = treasury;
        self.admin = admin;
    }

    pub fn get_config(&self) -> ConfigInfo {
        ConfigInfo {
            treasury: self.treasury.clone(),
            admin: self.admin.clone()
        }
    }

    #[payable]
    pub fn launch(
        &mut self,
        metadata: NFTContractMetadata,
        total_supply: U128,
        mint_price: U128,
        mint_currency: Option<AccountId>,
        royalty: U128,
        payment_split_percent: u32,
        burn_fee: u32,
        nft_id: String
    ) {
        let current_id = env::current_account_id();
        let owner = env::predecessor_account_id();
        let attached_deposit = env::attached_deposit().as_yoctonear();
        let code = include_bytes!("./nft/nft.wasm").to_vec();
        let contract_bytes = code.len() as u128;
        let byte_cost = env::storage_byte_cost();
        let minimum_needed = byte_cost.as_yoctonear() * contract_bytes + NFT_CONTRACT_STORAGE ;
        assert!(minimum_needed<=attached_deposit, "Must attach {} yoctoNEAR to cover storage", minimum_needed);
        let refund = attached_deposit - minimum_needed;
        require!(burn_fee+payment_split_percent < 100, "DS: Invalid burn fee and payment split percent");
        if refund > 1 {
            Promise::new(env::predecessor_account_id()).transfer(NearToken::from_yoctonear(refund));
        }
        // Deploy the nft contract
        let nft_contract_id: AccountId = format!("{}.{}", nft_id, current_id).parse().unwrap();

        Promise::new(nft_contract_id.clone())
            .create_account()
            .transfer(NearToken::from_yoctonear(minimum_needed))
            .deploy_contract(code)
            .function_call(
                "new".to_string(),
                if let Some(mint_currency) = mint_currency.clone() {
                    json!({
                        "owner_id": owner.to_string(),
                        "admin": self.admin.to_string(),
                        "metadata": metadata,
                        "total_supply": total_supply.0.to_string(),
                        "mint_price": mint_price.0.to_string(),
                        "mint_currency": mint_currency.to_string(),
                        "payment_split_percent": payment_split_percent.to_string(),
                        "burn_fee": burn_fee.to_string(),
                        "treasury": self.treasury.to_string(),
                        "royalty": royalty.0.to_string()
                    })
                } else {
                    json!({
                        "owner_id": owner.to_string(),
                        "admin": self.admin.to_string(),
                        "metadata": metadata,
                        "total_supply": total_supply.0.to_string(),
                        "mint_price": mint_price.0.to_string(),
                        "payment_split_percent": payment_split_percent.to_string(),
                        "burn_fee": burn_fee.to_string(),
                        "treasury": self.treasury.to_string(),
                        "royalty": royalty.0.to_string()
                    })
                }.to_string().into_bytes().to_vec(),
                NearToken::from_yoctonear(0),
                Gas::from_tgas(20)
            );
        
            Event::Launch {
                creator_id: &owner,
                collection_id: &nft_contract_id,
                total_supply: &total_supply,
                mint_price: &mint_price,
                mint_currency: mint_currency.as_ref(),
                name: &metadata.name,
                symbol: &metadata.symbol,
                base_uri: &metadata.base_uri,
                royalty: &royalty
            }
            .emit();
    }
}

#[derive(Serialize, Debug, Clone)]
#[serde(crate = "near_sdk::serde")]
#[serde(tag = "event", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum Event<'a> {
    Launch {
        creator_id: &'a AccountId,
        collection_id: &'a AccountId,
        total_supply: &'a U128,
        mint_price: &'a U128,
        name: &'a String,
        symbol: &'a String,
        royalty: &'a U128,
        #[serde(skip_serializing_if = "Option::is_none")]
        base_uri: &'a Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mint_currency: Option<&'a AccountId>,
    }
}

impl Event<'_> {
    pub fn emit(&self) {
        emit_event(&self);
    }
}

const EVENT_STANDARD: &str = "linear";
const EVENT_STANDARD_VERSION: &str = "1.0.0";

// Emit event that follows NEP-297 standard: https://nomicon.io/Standards/EventsFormat
// Arguments
// * `standard`: name of standard, e.g. nep171
// * `version`: e.g. 1.0.0
// * `event`: type of the event, e.g. nft_mint
// * `data`: associate event data. Strictly typed for each set {standard, version, event} inside corresponding NEP
pub(crate) fn emit_event<T: ?Sized + Serialize>(data: &T) {
    let result = json!(data);
    let event_json = json!({
        "standard": EVENT_STANDARD,
        "version": EVENT_STANDARD_VERSION,
        "event": result["event"],
        "data": [result["data"]]
    })
    .to_string();
    log!(format!("EVENT_JSON:{}", event_json));
}