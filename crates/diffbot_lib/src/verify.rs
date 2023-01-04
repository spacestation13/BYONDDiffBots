use hmac::{Hmac, Mac};
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

        //remove the `sha256=` part
        let (_, actual_signature) = sig.split_at(7);

        let mut hmac = HmacSha256::new_from_slice(sekrit.as_bytes()).unwrap();
        hmac.update(payload.as_bytes());
        if hmac.verify_slice(actual_signature.as_bytes()).is_err() {
            return Err(actix_web::error::ErrorBadRequest(
                "Signature does not match!",
            ));
        };
    }
    Ok(())
}
