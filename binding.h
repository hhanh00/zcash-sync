#ifndef __APPLE__
typedef char int8_t;
typedef unsigned char uint8_t;
typedef short int uint16_t;
typedef long long int int64_t;
typedef long long int uint64_t;
typedef long long int uintptr_t;
typedef long int int32_t;
typedef long int uint32_t;
typedef char bool;
#endif
typedef void *DartPostCObjectFnType;

#define QR_DATA_SIZE 256

#define N 200000

typedef struct CResult_u32 {
  uint32_t value;
  char *error;
} CResult_u32;

typedef struct CResult_u8 {
  uint8_t value;
  char *error;
} CResult_u8;

typedef struct CResult_____c_char {
  char *value;
  char *error;
} CResult_____c_char;

typedef struct CResult_i64 {
  int64_t value;
  char *error;
} CResult_i64;

typedef struct CResult_u64 {
  uint64_t value;
  char *error;
} CResult_u64;

void dummy_export(void);

void dart_post_cobject(DartPostCObjectFnType ptr);

void deallocate_str(char *s);

void init_wallet(char *db_path);

void set_active(uint8_t active);

void set_active_account(uint8_t coin, uint32_t id);

void set_coin_lwd_url(uint8_t coin, char *lwd_url);

char *get_lwd_url(uint8_t coin);

void reset_app(void);

struct CResult_u32 new_account(uint8_t coin, char *name, char *data, int32_t index);

void new_sub_account(char *name, int32_t index, uint32_t count);

void import_transparent_key(uint8_t coin, uint32_t id_account, char *path);

void import_transparent_secret_key(uint8_t coin, uint32_t id_account, char *secret_key);

void cancel_warp(void);

struct CResult_u8 warp(uint8_t coin,
                       bool get_tx,
                       uint32_t anchor_offset,
                       uint32_t max_cost,
                       int64_t port);

int8_t is_valid_key(uint8_t coin, char *key);

bool valid_address(uint8_t coin, char *address);

struct CResult_____c_char new_diversified_address(void);

struct CResult_u32 get_latest_height(void);

struct CResult_____c_char send_multi_payment(char *recipients_json,
                                             bool use_transparent,
                                             uint32_t anchor_offset,
                                             int64_t port);

void skip_to_last_height(uint8_t coin);

struct CResult_u32 rewind_to(uint32_t height);

void rescan_from(uint32_t height);

struct CResult_i64 mempool_sync(void);

void mempool_reset(void);

int64_t get_mempool_balance(void);

struct CResult_u64 get_taddr_balance(uint8_t coin, uint32_t id_account);

struct CResult_____c_char shield_taddr(void);

void scan_transparent_accounts(uint32_t gap_limit);

struct CResult_____c_char prepare_multi_payment(char *recipients_json,
                                                bool use_transparent,
                                                uint32_t anchor_offset);

struct CResult_____c_char sign(char *tx, int64_t port);

struct CResult_____c_char broadcast_tx(char *tx_str);

struct CResult_u32 get_activation_date(void);

struct CResult_u32 get_block_by_time(uint32_t time);

struct CResult_u32 sync_historical_prices(int64_t now, uint32_t days, char *currency);

void store_contact(uint32_t id, char *name, char *address, bool dirty);

struct CResult_____c_char commit_unsaved_contacts(uint32_t anchor_offset);

void mark_message_read(uint32_t message, bool read);

void mark_all_messages_read(bool read);

void truncate_data(void);

void truncate_sync_data(void);

void delete_account(uint8_t coin, uint32_t account);

struct CResult_____c_char make_payment_uri(char *address, uint64_t amount, char *memo);

struct CResult_____c_char parse_payment_uri(char *uri);

struct CResult_____c_char generate_random_enc_key(void);

struct CResult_____c_char get_full_backup(char *key);

void restore_full_backup(char *key, char *backup);

struct CResult_____c_char split_data(uint32_t id, char *data);

struct CResult_____c_char merge_data(char *drop);

struct CResult_____c_char get_tx_summary(char *tx);

struct CResult_____c_char get_best_server(char **servers, uint32_t count);

void import_from_zwl(uint8_t coin, char *name, char *data);

struct CResult_____c_char derive_zip32(uint8_t coin,
                                       uint32_t id_account,
                                       uint32_t account,
                                       uint32_t external,
                                       bool has_address,
                                       uint32_t address);

void disable_wal(char *db_path);

bool has_cuda(void);

bool has_metal(void);

bool has_gpu(void);

void use_gpu(bool v);

void import_sync_file(uint8_t coin, char *path);
