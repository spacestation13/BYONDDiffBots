use hmac::{
    digest::{generic_array::GenericArray, CtOutput},
    Hmac, Mac,
};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn verify_signature(
    secret: Option<&str>,
    signature: Option<&[u8]>,
    payload: &str,
) -> Result<(), actix_web::error::Error> {
    if let Some(sekrit) = secret {
        let Some(sig) = signature else {
            return Err(actix_web::error::ErrorBadRequest("Expected signature in header"))
        };

        log::trace!("Received sig: {:?}", sig);

        //have to wrap it to stop timing attacks on comparison
        let actual_signature = CtOutput::new(GenericArray::clone_from_slice(sig));

        let mut mac = HmacSha256::new_from_slice(sekrit.as_bytes()).unwrap();
        mac.update(payload.as_bytes());
        let computed_signature = mac.finalize();

        log::trace!(
            "Computed sig: {:?}",
            computed_signature.clone().into_bytes()
        );

        if actual_signature.ne(&computed_signature) {
            return Err(actix_web::error::ErrorBadRequest(
                "Signature does not match!",
            ));
        };
    }
    Ok(())
}
