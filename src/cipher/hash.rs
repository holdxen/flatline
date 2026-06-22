use openssl::{md::MdRef, md_ctx::MdCtx};
use snafu::ResultExt;

use crate::error::{Result, builder};

pub trait Hash {
    fn hash_len(&self) -> usize;
    fn update(&mut self, data: &[u8]) -> Result<()>;
    fn finalize(&mut self) -> Result<Vec<u8>>;
}

pub(crate) struct ReusableMd {
    ctx: MdCtx,
    ctxref: &'static MdRef,
}

impl ReusableMd {
    pub fn initialize(ctxref: &'static MdRef) -> Result<ReusableMd> {
        let mut ctx = MdCtx::new().context(builder::OpenSSL)?;

        ctx.digest_init(ctxref).context(builder::OpenSSL)?;

        Ok(ReusableMd { ctx, ctxref })
    }
}

impl Hash for ReusableMd {
    fn hash_len(&self) -> usize {
        self.ctx.size()
    }

    fn update(&mut self, data: &[u8]) -> Result<()> {
        self.ctx.digest_update(data).context(builder::OpenSSL)?;
        Ok(())
    }

    fn finalize(&mut self) -> Result<Vec<u8>> {
        let mut out = vec![0; self.ctx.size()];
        self.ctx.digest_final(&mut out).context(builder::OpenSSL)?;

        self.ctx
            .digest_init(self.ctxref)
            .context(builder::OpenSSL)?;

        Ok(out)
    }
}
