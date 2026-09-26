use crate::ai::service::GenerationInput;
use crate::ai::AIChatService;
use crate::gateway::whatsapp::delivery::WhatsAppDeliverySink;
use crate::gateway::whatsapp::mapper::{
    build_inbound_envelope, is_sender_authorized, parse_group_jid_to_i64, parse_user_jid_to_i64,
};
use crate::gateway::whatsapp::WhatsAppConfig;
use crate::gateway::{DeliverySink, InboundEnvelope};
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use tracing::{error, info, warn};
use whatsapp_rust::pair_code::PairCodeOptions;
use whatsapp_rust::prelude::*;

pub struct WhatsAppClientRunner;

impl WhatsAppClientRunner {
    pub async fn run(
        config: WhatsAppConfig,
        ai_service: Arc<AIChatService>,
        inbound_tx: Option<mpsc::Sender<InboundEnvelope>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let db_str = config.db_path.to_string_lossy().to_string();
        let store = SqliteStore::new(&db_str).await?;
        info!("WhatsApp SQLite session store loaded at: {db_str}");

        let mut builder = Bot::builder()
            .with_backend(store)
            .on_qr_code(|code, timeout| async move {
                println!("\n  \x1b[38;2;16;185;129m╭──────────────────────────────────────────────────────────╮\x1b[0m");
                println!("  \x1b[38;2;16;185;129m│\x1b[0m \x1b[1;37m📲 SCAN QR CODE WHATSAPP (Batas: {}s)\x1b[0m                    \x1b[38;2;16;185;129m│\x1b[0m", timeout.as_secs());
                println!("  \x1b[38;2;16;185;129m╰──────────────────────────────────────────────────────────╯\x1b[0m\n");
                if let Err(e) = qr2term::print_qr(&code) {
                    error!("Gagal render QR visual: {e}. Raw code: {code}");
                }
                println!("\n  \x1b[38;5;244mBuka WhatsApp HP > Perangkat Tertaut > Tautkan Perangkat\x1b[0m\n");
            })
            .on_pair_code(|code, timeout| async move {
                println!("\n  \x1b[1;32m🔑 KODE PAIRING WHATSAPP (Batas: {}s): \x1b[1;37m>>> \x1b[1;33m{}\x1b[1;37m <<<\x1b[0m", timeout.as_secs(), code);
                println!("  \x1b[38;5;244mBuka WhatsApp HP > Perangkat Tertaut > Tautkan dengan nomor telepon\x1b[0m\n");
            })
            .on_connected(|_client| async {
                info!("Berhasil terhubung ke jaringan WhatsApp! Gateway siap aktif.");
                println!("\n  \x1b[1;32m●\x1b[0m \x1b[1;37mWhatsApp Gateway: Terhubung dan Aktif!\x1b[0m\n");
            })
            .on_logged_out(|_info| async {
                warn!("Sesi WhatsApp telah dikeluarkan / di-logout.");
                println!("\n  \x1b[31m✖ Sesi WhatsApp telah dikeluarkan / logout.\x1b[0m\n");
            });

        let owner_number = config.owner_number.clone();
        let ai = Arc::clone(&ai_service);
        let tx_opt = inbound_tx.clone();
        // Bounded concurrency pool (maksimal 8 generasi paralel, konsisten dengan worker.rs)
        let concurrency_semaphore = Arc::new(Semaphore::new(8));

        builder = builder.on_message(move |ctx| {
            let owner_opt = owner_number.clone();
            let ai_ref = Arc::clone(&ai);
            let tx = tx_opt.clone();
            let semaphore = Arc::clone(&concurrency_semaphore);

            async move {
                if ctx.info.source.is_from_me {
                    return;
                }

                let sender_jid_str = ctx.info.source.sender.to_string();
                let Some(sender_id) = parse_user_jid_to_i64(&sender_jid_str) else {
                    return;
                };

                // HARDENED SINGLE-OWNER BOUNDARY:
                // Pesan dari nomor yang tidak berwenang diabaikan seketika (silent drop)
                // tanpa mencatat ID pengirim ke log (zero information leakage).
                if !is_sender_authorized(sender_id, owner_opt.as_deref()) {
                    return;
                }

                let chat_jid_str = ctx.info.source.chat.to_string();
                let is_group = ctx.info.source.is_group;
                let chat_id = if is_group {
                    parse_group_jid_to_i64(&chat_jid_str)
                } else {
                    sender_id
                };

                let raw_text = ctx.message.text_content().unwrap_or_default();
                let trimmed = raw_text.trim();

                // Deteksi lampiran media
                let base = ctx.message.get_base_message();
                let has_media = base.image_message.as_option().is_some()
                    || base.audio_message.as_option().is_some()
                    || base.document_message.as_option().is_some();

                if trimmed.is_empty() && !has_media {
                    return;
                }

                info!("💬 [WA INTAKE] dari [{sender_id}] di [{chat_id}]: {trimmed}");

                // Salurkan ke InboundEnvelope jika caller mendengarkan via channel
                if let Some(ref tx_chan) = tx {
                    let env = build_inbound_envelope(
                        sender_id,
                        chat_id,
                        trimmed.to_string(),
                        None,
                        is_group,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    );
                    let _ = tx_chan.send(env).await;
                }

                // Inisialisasi WhatsAppDeliverySink untuk menangani output
                let delivery_sink = WhatsAppDeliverySink::new(ctx.client.clone());
                delivery_sink
                    .remember_jid(chat_id, ctx.info.source.chat.clone())
                    .await;

                // Batasi concurrency dengan semaphore agar tidak membebani sistem
                let Ok(permit) = semaphore.acquire_owned().await else {
                    return;
                };

                // Indikator sedang mengetik via DeliverySink
                let _ = delivery_sink.indicate_typing(chat_id).await;

                let (_cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
                let gen_input = GenerationInput {
                    prompt: trimmed,
                    canonical_prompt: None,
                    media_to_main: false,
                    sink: None,
                    image_bytes: None,
                    document_images: None,
                    mime_type: None,
                    doc_text: None,
                    doc_name: None,
                    audio_bytes: None,
                    audio_mime: None,
                    video_bytes: None,
                    video_mime: None,
                    video_duration: None,
                    bot: None,
                    reply_to_message_id: None,
                };

                let res = ai_ref
                    .generate_response(sender_id, 0, chat_id, gen_input, &mut cancel_rx)
                    .await;
                let (_thinking, answer, _staged_docs, cancelled) = res;

                if !cancelled && !answer.trim().is_empty() {
                    let _ = delivery_sink.send_text(chat_id, &answer, None).await;
                }

                delivery_sink.stop_typing(chat_id).await;
                drop(permit);
            }
        });

        if let Some(phone) = config.phone_login {
            info!("Mengaktifkan pairing kode telepon: {phone}");
            builder = builder.with_pair_code(PairCodeOptions {
                phone_number: phone,
                ..Default::default()
            });
        }

        let bot = builder.build().await?;
        info!("Client WhatsApp terinisialisasi. Menghubungkan...");

        let mut handle = bot.spawn();

        tokio::select! {
            res = &mut handle => {
                info!("Loop WhatsApp bot selesai: {:?}", res);
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Sinyal shutdown diterima, menghentikan WhatsApp client...");
                handle.shutdown().await;
            }
        }

        Ok(())
    }
}
