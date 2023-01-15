use hmac::{digest::CtOutput, Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn verify_signature(
    secret: Option<&str>,
    signature: Option<&str>,
    payload: &str,
) -> Result<(), actix_web::error::Error> {
    if let Some(sekrit) = secret {
        let Some(sig) = signature else {
            return Err(actix_web::error::ErrorBadRequest("Expected signature in header"))
        };

        log::trace!("Signature received: {}", sig.clone());

        //remove the `sha256=` part
        let (_, actual_signature) = sig.split_at(7);

        log::trace!("Truncated sig: {}", actual_signature.clone());

        //have to wrap it to stop timing attacks on comparison
        let actual_signature = CtOutput::new(actual_signature.as_bytes());

        let mut computed_signature = HmacSha256::new_from_slice(sekrit.as_bytes())
            .unwrap()
            .update(payload.as_bytes())
            .finalize();

        log::trace!("Computed sig: {}", computed_signature.clone());

        if !actual_signature.ct_eq(computed_signature) {
            return Err(actix_web::error::ErrorBadRequest(
                "Signature does not match!",
            ));
        };
    }
    Ok(())
}
