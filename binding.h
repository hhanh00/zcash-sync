#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

void dart_post_cobject(DartPostCObjectFnType ptr);

void init_wallet(char *db_path);

void set_active(uint8_t active);

void set_active_account(uint8_t coin, uint32_t id);

void set_coin_lwd_url(uint8_t coin, char *lwd_url);

void reset_app(void);

uint32_t new_account(uint8_t coin, char *name, char *data, int32_t index);

uint32_t new_sub_account(char *name, int32_t index);

uint8_t warp(uint8_t coin, bool get_tx, uint32_t anchor_offset, int64_t port);

int8_t is_valid_key(uint8_t coin, char *key);

bool valid_address(uint8_t coin, char *address);

char *new_diversified_address(void);

uint32_t get_latest_height(void);

char *send_multi_payment(char *recipients_json,
                         bool use_transparent,
                         uint32_t anchor_offset,
                         int64_t port);

void skip_to_last_height(uint8_t coin);

void rewind_to_height(uint32_t height);

int64_t mempool_sync(void);

void mempool_reset(void);

int64_t get_mempool_balance(void);

uint64_t get_taddr_balance(uint8_t coin, uint32_t id_account);

char *shield_taddr(void);

void scan_transparent_accounts(uint32_t gap_limit);

char *prepare_multi_payment(char *recipients_json, bool use_transparent, uint32_t anchor_offset);

char *sign(char *tx_filename, int64_t port);

char *broadcast(char *tx_filename);

char *broadcast_txhex(char *txhex);

uint32_t get_activation_date(void);

uint32_t get_block_by_time(uint32_t time);

uint32_t sync_historical_prices(int64_t now, uint32_t days, char *currency);

void store_contact(uint32_t id, char *name, char *address, bool dirty);

char *commit_unsaved_contacts(uint32_t anchor_offset);

void mark_message_read(uint32_t message, bool read);

void mark_all_messages_read(bool read);

void truncate_data(void);

void delete_account(uint8_t coin, uint32_t account);

char *make_payment_uri(char *address, uint64_t amount, char *memo);

char *parse_payment_uri(char *uri);

char *generate_random_enc_key(void);

char *get_full_backup(char *key);

char *restore_full_backup(char *key, char *backup);
