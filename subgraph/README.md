# Near-Marketplace subgraph

Indexes the three contracts in this repo into a GraphQL API:

| Data source   | Account                                | What it reads                                                             |
| ------------- | -------------------------------------- | ------------------------------------------------------------------------- |
| `Marketplace` | the marketplace contract               | `add_market_data`, `delete_market_data`, `resolve_purchase`, `add_bid`, `cancel_bid`, `extend_auction` logs + `buy` / `accept_bid` calls |
| `Launchpad`   | the launchpad contract                 | NEP-297 `launch` events → `Collection`                                    |
| `Collections` | every `*.<launchpad>` sub-account      | NEP-171 `nft_mint` / `nft_transfer` + `burn`, `set_mint_type`, `set_collection_owner` calls |

## Before deploying — fill in `subgraph.yaml`

Three values are placeholders:

- `Marketplace.source.account` — the deployed marketplace account id
- `Launchpad.source.account` — the launchpad account id. It is inferred from the
  test fixtures (`shardsquares.factory.defishards.near` ⇒ `factory.defishards.near`);
  confirm it.
- `Collections.source.accounts.suffixes` — must be `.<launchpad account>`
- `startBlock` on all three — the deployment blocks. Anything earlier just wastes
  indexing time; anything later silently loses history.

For testnet, also switch `network` to `near-testnet` on all three data sources.

## Build and deploy

```bash
npm install
npm run codegen
npm run build

# Subgraph Studio (https://thegraph.com/studio) — create the subgraph first,
# then authenticate with the deploy key it gives you:
npx graph auth <deploy-key>
npx graph deploy <subgraph-slug>
```

NEAR indexing on The Graph runs through a Firehose, not a plain NEAR RPC node.
Subgraph Studio and the upgrade Indexer provide that; a self-hosted `graph-node`
additionally needs a NEAR Firehose endpoint, so Studio is the practical target.
NEAR support is still labelled beta on The Graph Network.

## The frontend query

`marketStatus` is a `String`, not a GraphQL enum, so the existing query works
verbatim — an enum would have rejected the quoted `"sale"`:

```js
const getMarketDataByCollection = useCallback(
  async (collection_id, priceOrder, skip = 0, limit = 12) => {
    const query = `{
      nfts(
        where: { marketStatus: "sale", collection: "${collection_id}" }
        orderBy: price
        orderDirection: ${priceOrder ? 'desc' : 'asc'}
        skip: ${skip}
        first: ${limit}
      ) {
        id
        tokenId
        price
        isAuction
        endPrice
        endedAt
        seller
        title
        media
        owner { id }
      }
    }`;
    const {
      data: {
        data: { nfts },
      },
    } = await axios.post(GraphURL, { query });
    return nfts;
  },
  []
);
```

- `Nft.id` is `"<nft_contract_id>||<token_id>"`, the same key the marketplace
  contract uses for `ContractAndTokenId`. `tokenId` is available separately.
- `collection` is the NFT contract account id.
- `marketStatus` is `"sale"` while listed (fixed price **and** auction) and
  `"none"` otherwise. Filter fixed-price only with `isAuction: false`.
- `price` is a `BigInt` in yoctoNEAR, `0` when unlisted, so `orderBy: price`
  sorts numerically rather than lexicographically.

## Known gaps

These are limits of what the contracts emit, not of the mappings:

- **Dutch auctions.** `get_market_data` recomputes a decaying price from
  `block_timestamp`. A subgraph cannot evaluate time-varying values, so `price`
  stays at the starting price and `endPrice` / `startedAt` / `endedAt` are
  exposed for the frontend to interpolate.
- **`add_bid` `ended_at`.** The log field named `ended_at` actually carries the
  bid timestamp; the contract stores `block_timestamp + FIVE_MINUTES`. The
  mapping reconstructs the stored value. Note the contract's unconditional
  `ended_at = current_time + FIVE_MINUTES` on every bid also overwrites the
  extension emitted by `extend_auction` in the same call — the mapping mirrors
  on-chain state, extension log included, in log order.
- **Failed purchases.** `internal_process_purchase` deletes the market data
  before calling `nft_transfer_payout` and emits nothing if the transfer fails.
  The mapping unlists from the `buy` / `accept_bid` call itself so the listing
  disappears either way.
- **Burns.** `Contract::burn` does not emit NEP-171 `nft_burn`, so burns are
  read from the function call arguments. A `nft_burn` handler is wired up in
  case the contract starts emitting one.
- **Collections outside the launchpad.** The marketplace can approve externally
  deployed NFT contracts (e.g. one whose account is not a `*.<launchpad>`
  sub-account). Listings and sales for those are still indexed, but mints,
  transfers and metadata are not — add another `kind: near` data source per
  extra contract. NEAR subgraphs have no data source templates, so this cannot
  be done dynamically.
- **Floor price** is maintained incrementally and is set to `null` when the
  cheapest listing is removed; the next listing re-seeds it. Treat `floorPrice`
  as best-effort.
- **`Account.totalSpent` / `totalEarned`** are gross sale prices; the treasury
  fee and royalty split are computed inside `resolve_purchase` and never logged.
