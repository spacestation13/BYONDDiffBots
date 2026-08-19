use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn verify_signature(
    secret: Option<&str>,
    signature: Option<&[u8]>,
    payload: &str,
) -> Result<(), actix_web::error::Error> {
    if let Some(sekrit) = secret {
        let Some(sig) = signature else {
            return Err(actix_web::error::ErrorBadRequest(
                "Expected signature in header",
            ));
        };

        let mut mac = HmacSha256::new_from_slice(sekrit.as_bytes()).unwrap();
        mac.update(payload.as_bytes());

        match mac.verify_slice(sig) {
            Ok(_) => {}
            Err(e) => return Err(actix_web::error::ErrorBadRequest(e)),
        }
    }
    Ok(())
}
