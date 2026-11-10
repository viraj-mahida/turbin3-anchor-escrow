use {
    anchor_lang::{
        prelude::Pubkey,
        solana_program::{instruction::Instruction, system_program},
        AccountDeserialize, InstructionData, ToAccountMetas,
    },
    anchor_spl::{
        associated_token::{self, get_associated_token_address_with_program_id},
        token::{
            self,
            spl_token::state::{Account as SplAccount, AccountState, Mint as SplMint},
        },
        token_2022,
    },
    escrow::{Escrow, ESCROW_SEED},
    litesvm::{types::FailedTransactionMetadata, LiteSVM},
    solana_account::Account,
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_program_option::COption,
    solana_program_pack::Pack,
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
};

const DECIMALS: u8 = 6;
const MAKER_A: u64 = 1_000_000_000;
const TAKER_B: u64 = 1_000_000_000;
const DEPOSIT: u64 = 250_000_000;
const RECEIVE: u64 = 400_000_000;
const SEED: u64 = 42;

struct Ctx {
    svm: LiteSVM,
    token_program: Pubkey,
    maker: Keypair,
    taker: Keypair,
    mint_a: Pubkey,
    mint_b: Pubkey,
    maker_ata_a: Pubkey,
    maker_ata_b: Pubkey,
    taker_ata_a: Pubkey,
    taker_ata_b: Pubkey,
    escrow: Pubkey,
    vault: Pubkey,
}

// ---------------------------------------------------------------- helpers --

fn ata(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    get_associated_token_address_with_program_id(owner, mint, token_program)
}

fn write_mint(svm: &mut LiteSVM, mint: Pubkey, authority: Pubkey, token_program: Pubkey) {
    let mut data = vec![0u8; SplMint::LEN];
    SplMint::pack(
        SplMint {
            mint_authority: COption::Some(authority),
            supply: u64::MAX / 2,
            decimals: DECIMALS,
            is_initialized: true,
            freeze_authority: COption::None,
        },
        &mut data,
    )
    .unwrap();
    let lamports = svm.minimum_balance_for_rent_exemption(data.len());
    svm.set_account(
        mint,
        Account {
            lamports,
            data,
            owner: token_program,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

fn write_token_account(
    svm: &mut LiteSVM,
    address: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    amount: u64,
    token_program: Pubkey,
) {
    let mut data = vec![0u8; SplAccount::LEN];
    SplAccount::pack(
        SplAccount {
            mint,
            owner,
            amount,
            delegate: COption::None,
            state: AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 0,
            close_authority: COption::None,
        },
        &mut data,
    )
    .unwrap();
    let lamports = svm.minimum_balance_for_rent_exemption(data.len());
    svm.set_account(
        address,
        Account {
            lamports,
            data,
            owner: token_program,
            executable: false,
            rent_epoch: 0,
        },
    )
    .unwrap();
}

fn token_balance(svm: &LiteSVM, address: &Pubkey) -> u64 {
    let account = svm
        .get_account(address)
        .expect("token account should exist");
    SplAccount::unpack(&account.data[..SplAccount::LEN])
        .unwrap()
        .amount
}

fn is_closed(svm: &LiteSVM, address: &Pubkey) -> bool {
    svm.get_account(address)
        .map_or(true, |a| a.lamports == 0 && a.data.is_empty())
}

fn read_escrow(svm: &LiteSVM, address: &Pubkey) -> Escrow {
    let account = svm.get_account(address).expect("escrow should exist");
    Escrow::try_deserialize(&mut account.data.as_slice()).unwrap()
}

fn try_send(
    svm: &mut LiteSVM,
    payer: &Keypair,
    ix: Instruction,
) -> Result<(), FailedTransactionMetadata> {
    svm.expire_blockhash();
    let blockhash = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(&[ix], Some(&payer.pubkey()), &blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), &[payer]).unwrap();
    svm.send_transaction(tx).map(|_| ())
}

fn send(svm: &mut LiteSVM, payer: &Keypair, ix: Instruction) {
    try_send(svm, payer, ix).unwrap_or_else(|err| {
        panic!("tx failed: {err:?}\nlogs: {logs:#?}", logs = err.meta.logs);
    });
}

fn assert_fails_with(result: Result<(), FailedTransactionMetadata>, needle: &str) {
    let err = result.expect_err("transaction should have failed");
    let logs = err.meta.logs.join("\n");
    assert!(
        logs.contains(needle),
        "expected logs to contain `{needle}`, got:\n{logs}"
    );
}

fn setup_with(token_program: Pubkey) -> Ctx {
    let mut svm = LiteSVM::new();
    let bytes = include_bytes!(concat!(env!("CARGO_TARGET_TMPDIR"), "/../deploy/escrow.so"));
    svm.add_program(escrow::id(), bytes).unwrap();

    let maker = Keypair::new();
    let taker = Keypair::new();
    svm.airdrop(&maker.pubkey(), 10_000_000_000).unwrap();
    svm.airdrop(&taker.pubkey(), 10_000_000_000).unwrap();

    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    let mint_authority = Pubkey::new_unique();
    write_mint(&mut svm, mint_a, mint_authority, token_program);
    write_mint(&mut svm, mint_b, mint_authority, token_program);

    let maker_ata_a = ata(&maker.pubkey(), &mint_a, &token_program);
    let maker_ata_b = ata(&maker.pubkey(), &mint_b, &token_program);
    let taker_ata_a = ata(&taker.pubkey(), &mint_a, &token_program);
    let taker_ata_b = ata(&taker.pubkey(), &mint_b, &token_program);
    write_token_account(
        &mut svm,
        maker_ata_a,
        mint_a,
        maker.pubkey(),
        MAKER_A,
        token_program,
    );
    write_token_account(
        &mut svm,
        taker_ata_b,
        mint_b,
        taker.pubkey(),
        TAKER_B,
        token_program,
    );

    let (escrow, _) = Pubkey::find_program_address(
        &[ESCROW_SEED, maker.pubkey().as_ref(), &SEED.to_le_bytes()],
        &escrow::id(),
    );
    let vault = ata(&escrow, &mint_a, &token_program);

    Ctx {
        svm,
        token_program,
        maker,
        taker,
        mint_a,
        mint_b,
        maker_ata_a,
        maker_ata_b,
        taker_ata_a,
        taker_ata_b,
        escrow,
        vault,
    }
}

fn setup() -> Ctx {
    setup_with(token::ID)
}

// ------------------------------------------------------- instruction builders --

fn make_ix(c: &Ctx, maker: Pubkey, mint_b: Pubkey, deposit: u64, receive: u64) -> Instruction {
    let maker_ata_a = ata(&maker, &c.mint_a, &c.token_program);
    let (escrow, _) = Pubkey::find_program_address(
        &[ESCROW_SEED, maker.as_ref(), &SEED.to_le_bytes()],
        &escrow::id(),
    );
    Instruction::new_with_bytes(
        escrow::id(),
        &escrow::instruction::Make {
            seed: SEED,
            deposit,
            receive,
        }
        .data(),
        escrow::accounts::Make {
            maker,
            mint_a: c.mint_a,
            mint_b,
            maker_ata_a,
            escrow,
            vault: ata(&escrow, &c.mint_a, &c.token_program),
            associated_token_program: associated_token::ID,
            token_program: c.token_program,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

fn take_ix(c: &Ctx) -> Instruction {
    Instruction::new_with_bytes(
        escrow::id(),
        &escrow::instruction::Take {}.data(),
        escrow::accounts::Take {
            taker: c.taker.pubkey(),
            maker: c.maker.pubkey(),
            mint_a: c.mint_a,
            mint_b: c.mint_b,
            taker_ata_a: c.taker_ata_a,
            taker_ata_b: c.taker_ata_b,
            maker_ata_b: c.maker_ata_b,
            escrow: c.escrow,
            vault: c.vault,
            associated_token_program: associated_token::ID,
            token_program: c.token_program,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

fn refund_ix(c: &Ctx, maker: Pubkey) -> Instruction {
    Instruction::new_with_bytes(
        escrow::id(),
        &escrow::instruction::Refund {}.data(),
        escrow::accounts::Refund {
            maker,
            mint_a: c.mint_a,
            maker_ata_a: ata(&maker, &c.mint_a, &c.token_program),
            escrow: c.escrow,
            vault: c.vault,
            associated_token_program: associated_token::ID,
            token_program: c.token_program,
            system_program: system_program::ID,
        }
        .to_account_metas(None),
    )
}

fn update_ix(c: &Ctx, maker: Pubkey, new_receive: u64, new_deposit: Option<u64>) -> Instruction {
    Instruction::new_with_bytes(
        escrow::id(),
        &escrow::instruction::Update {
            new_receive,
            new_deposit,
        }
        .data(),
        escrow::accounts::Update {
            maker,
            mint_a: c.mint_a,
            maker_ata_a: ata(&maker, &c.mint_a, &c.token_program),
            escrow: c.escrow,
            vault: c.vault,
            token_program: c.token_program,
        }
        .to_account_metas(None),
    )
}

fn made_with(token_program: Pubkey) -> Ctx {
    let mut c = setup_with(token_program);
    let ix = make_ix(&c, c.maker.pubkey(), c.mint_b, DEPOSIT, RECEIVE);
    let maker = c.maker.insecure_clone();
    send(&mut c.svm, &maker, ix);
    c
}

fn made() -> Ctx {
    made_with(token::ID)
}

fn funded_attacker(c: &mut Ctx) -> Keypair {
    let attacker = Keypair::new();
    c.svm.airdrop(&attacker.pubkey(), 10_000_000_000).unwrap();
    let attacker_ata_a = ata(&attacker.pubkey(), &c.mint_a, &c.token_program);
    write_token_account(
        &mut c.svm,
        attacker_ata_a,
        c.mint_a,
        attacker.pubkey(),
        0,
        c.token_program,
    );
    attacker
}

// ------------------------------------------------------------------ make --

#[test]
fn make_locks_deposit_and_records_terms() {
    let c = made();

    assert_eq!(
        token_balance(&c.svm, &c.vault),
        DEPOSIT,
        "vault holds the deposit"
    );
    assert_eq!(token_balance(&c.svm, &c.maker_ata_a), MAKER_A - DEPOSIT);

    let state = read_escrow(&c.svm, &c.escrow);
    assert_eq!(state.seed, SEED);
    assert_eq!(state.maker, c.maker.pubkey());
    assert_eq!(state.mint_a, c.mint_a);
    assert_eq!(state.mint_b, c.mint_b);
    assert_eq!(state.receive, RECEIVE);

    let vault = c.svm.get_account(&c.vault).unwrap();
    let vault = SplAccount::unpack(&vault.data).unwrap();
    assert_eq!(vault.owner, c.escrow, "vault is owned by the escrow PDA");
}

#[test]
fn make_with_zero_deposit_fails() {
    let mut c = setup();
    let ix = make_ix(&c, c.maker.pubkey(), c.mint_b, 0, RECEIVE);
    let maker = c.maker.insecure_clone();
    assert_fails_with(try_send(&mut c.svm, &maker, ix), "InvalidAmount");
}

#[test]
fn make_with_zero_receive_fails() {
    let mut c = setup();
    let ix = make_ix(&c, c.maker.pubkey(), c.mint_b, DEPOSIT, 0);
    let maker = c.maker.insecure_clone();
    assert_fails_with(try_send(&mut c.svm, &maker, ix), "InvalidAmount");
}

#[test]
fn make_with_same_mint_fails() {
    let mut c = setup();
    let ix = make_ix(&c, c.maker.pubkey(), c.mint_a, DEPOSIT, RECEIVE);
    let maker = c.maker.insecure_clone();
    assert_fails_with(try_send(&mut c.svm, &maker, ix), "SameMint");
}

#[test]
fn make_twice_with_same_seed_fails() {
    let mut c = made();
    let ix = make_ix(&c, c.maker.pubkey(), c.mint_b, DEPOSIT, RECEIVE);
    let maker = c.maker.insecure_clone();
    assert_fails_with(try_send(&mut c.svm, &maker, ix), "already in use");
}

// ------------------------------------------------------------------ take --

#[test]
fn take_swaps_tokens_and_closes_accounts() {
    let mut c = made();
    let maker_sol_before = c.svm.get_balance(&c.maker.pubkey()).unwrap();
    let escrow_rent = c.svm.get_balance(&c.escrow).unwrap();
    let vault_rent = c.svm.get_balance(&c.vault).unwrap();

    let ix = take_ix(&c);
    let taker = c.taker.insecure_clone();
    send(&mut c.svm, &taker, ix);

    assert_eq!(
        token_balance(&c.svm, &c.taker_ata_a),
        DEPOSIT,
        "taker gets mint A"
    );
    assert_eq!(token_balance(&c.svm, &c.taker_ata_b), TAKER_B - RECEIVE);
    assert_eq!(
        token_balance(&c.svm, &c.maker_ata_b),
        RECEIVE,
        "maker gets mint B"
    );

    assert!(is_closed(&c.svm, &c.escrow), "escrow should be closed");
    assert!(is_closed(&c.svm, &c.vault), "vault should be closed");
    assert_eq!(
        c.svm.get_balance(&c.maker.pubkey()).unwrap(),
        maker_sol_before + escrow_rent + vault_rent,
        "maker gets the rent of both closed accounts"
    );
}

#[test]
fn take_without_enough_mint_b_fails() {
    let mut c = made();
    write_token_account(
        &mut c.svm,
        c.taker_ata_b,
        c.mint_b,
        c.taker.pubkey(),
        RECEIVE - 1,
        c.token_program,
    );
    let ix = take_ix(&c);
    let taker = c.taker.insecure_clone();
    assert_fails_with(try_send(&mut c.svm, &taker, ix), "insufficient funds");
    assert_eq!(
        token_balance(&c.svm, &c.vault),
        DEPOSIT,
        "vault is untouched"
    );
}

#[test]
fn take_twice_fails() {
    let mut c = made();
    let taker = c.taker.insecure_clone();
    let ix = take_ix(&c);
    send(&mut c.svm, &taker, ix);
    let ix = take_ix(&c);
    assert!(try_send(&mut c.svm, &taker, ix).is_err());
}

#[test]
fn take_after_refund_fails() {
    let mut c = made();
    let maker = c.maker.insecure_clone();
    let ix = refund_ix(&c, maker.pubkey());
    send(&mut c.svm, &maker, ix);

    let ix = take_ix(&c);
    let taker = c.taker.insecure_clone();
    assert_fails_with(try_send(&mut c.svm, &taker, ix), "AccountNotInitialized");
    assert_eq!(
        token_balance(&c.svm, &c.taker_ata_b),
        TAKER_B,
        "taker keeps mint B"
    );
}

#[test]
fn take_after_update_pays_the_new_price() {
    let mut c = made();
    let maker = c.maker.insecure_clone();
    let new_receive = RECEIVE / 2;
    let ix = update_ix(&c, maker.pubkey(), new_receive, None);
    send(&mut c.svm, &maker, ix);

    let ix = take_ix(&c);
    let taker = c.taker.insecure_clone();
    send(&mut c.svm, &taker, ix);

    assert_eq!(token_balance(&c.svm, &c.maker_ata_b), new_receive);
    assert_eq!(token_balance(&c.svm, &c.taker_ata_b), TAKER_B - new_receive);
    assert_eq!(token_balance(&c.svm, &c.taker_ata_a), DEPOSIT);
}

// ---------------------------------------------------------------- refund --

#[test]
fn refund_returns_deposit_and_closes_accounts() {
    let mut c = made();
    let maker = c.maker.insecure_clone();
    let ix = refund_ix(&c, maker.pubkey());
    send(&mut c.svm, &maker, ix);

    assert_eq!(
        token_balance(&c.svm, &c.maker_ata_a),
        MAKER_A,
        "maker is made whole"
    );
    assert!(is_closed(&c.svm, &c.escrow), "escrow should be closed");
    assert!(is_closed(&c.svm, &c.vault), "vault should be closed");
}

#[test]
fn refund_by_wrong_signer_fails() {
    let mut c = made();
    let attacker = funded_attacker(&mut c);
    let ix = refund_ix(&c, attacker.pubkey());
    assert_fails_with(try_send(&mut c.svm, &attacker, ix), "Unauthorized");

    assert_eq!(
        token_balance(&c.svm, &c.vault),
        DEPOSIT,
        "vault is untouched"
    );
    assert!(!is_closed(&c.svm, &c.escrow));
}

// ---------------------------------------------------------------- update --

#[test]
fn update_changes_receive_amount() {
    let mut c = made();
    let maker = c.maker.insecure_clone();
    let ix = update_ix(&c, maker.pubkey(), RECEIVE * 2, None);
    send(&mut c.svm, &maker, ix);

    assert_eq!(read_escrow(&c.svm, &c.escrow).receive, RECEIVE * 2);
    assert_eq!(
        token_balance(&c.svm, &c.vault),
        DEPOSIT,
        "deposit unchanged"
    );
}

#[test]
fn update_can_top_up_deposit() {
    let mut c = made();
    let maker = c.maker.insecure_clone();
    let new_deposit = DEPOSIT * 3;
    let ix = update_ix(&c, maker.pubkey(), RECEIVE, Some(new_deposit));
    send(&mut c.svm, &maker, ix);

    assert_eq!(token_balance(&c.svm, &c.vault), new_deposit);
    assert_eq!(token_balance(&c.svm, &c.maker_ata_a), MAKER_A - new_deposit);
}

#[test]
fn update_can_shrink_deposit() {
    let mut c = made();
    let maker = c.maker.insecure_clone();
    let new_deposit = DEPOSIT / 5;
    let ix = update_ix(&c, maker.pubkey(), RECEIVE, Some(new_deposit));
    send(&mut c.svm, &maker, ix);

    assert_eq!(token_balance(&c.svm, &c.vault), new_deposit);
    assert_eq!(token_balance(&c.svm, &c.maker_ata_a), MAKER_A - new_deposit);
}

#[test]
fn update_with_zero_receive_fails() {
    let mut c = made();
    let maker = c.maker.insecure_clone();
    let ix = update_ix(&c, maker.pubkey(), 0, None);
    assert_fails_with(try_send(&mut c.svm, &maker, ix), "InvalidAmount");
}

#[test]
fn update_by_wrong_signer_fails() {
    let mut c = made();
    let attacker = funded_attacker(&mut c);
    let ix = update_ix(&c, attacker.pubkey(), 1, Some(1));
    assert_fails_with(try_send(&mut c.svm, &attacker, ix), "Unauthorized");

    assert_eq!(
        read_escrow(&c.svm, &c.escrow).receive,
        RECEIVE,
        "terms unchanged"
    );
    assert_eq!(
        token_balance(&c.svm, &c.vault),
        DEPOSIT,
        "vault is untouched"
    );
}

// ------------------------------------------------------------ token-2022 --

#[test]
fn full_flow_works_with_token_2022() {
    let mut c = made_with(token_2022::ID);
    assert_eq!(token_balance(&c.svm, &c.vault), DEPOSIT);

    let ix = take_ix(&c);
    let taker = c.taker.insecure_clone();
    send(&mut c.svm, &taker, ix);

    assert_eq!(token_balance(&c.svm, &c.taker_ata_a), DEPOSIT);
    assert_eq!(token_balance(&c.svm, &c.maker_ata_b), RECEIVE);
    assert!(is_closed(&c.svm, &c.escrow));
    assert!(is_closed(&c.svm, &c.vault));
}
