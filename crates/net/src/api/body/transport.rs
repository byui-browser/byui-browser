//! Transport and scheduler integration for Fetch bodies.

use bytes::Bytes;
use reqwest::Body as ReqwestBody;

use crate::api::error::RequestError;

use super::{Body, BodyPhase, BodySource, BoxedBodyStream};

impl Body {
    /// Transfers the source to Reqwest for a request send.
    pub(crate) fn into_reqwest_body(self) -> Result<ReqwestBody, RequestError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RequestError::BodyAlreadyConsumed)?;
        if state.lifecycle.phase != BodyPhase::Unused {
            return Err(RequestError::BodyAlreadyConsumed);
        }
        state.lifecycle.phase = BodyPhase::Transferred;
        match &mut state.source {
            BodySource::Null => Ok(ReqwestBody::from(Bytes::new())),
            BodySource::Replayable(bytes) => Ok(ReqwestBody::from(bytes.clone())),
            BodySource::OneShotBytes(bytes) => bytes
                .take()
                .map(ReqwestBody::from)
                .ok_or(RequestError::BodyAlreadyConsumed),
            BodySource::Stream(stream) => stream
                .take()
                .map(ReqwestBody::wrap_stream)
                .ok_or(RequestError::BodyAlreadyConsumed),
        }
    }

    /// Separates a live stream from its scheduler permit for internal wrappers.
    pub(crate) fn try_into_parts(
        self,
    ) -> Result<(BoxedBodyStream, Option<tokio::sync::OwnedSemaphorePermit>), RequestError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RequestError::BodyAlreadyConsumed)?;
        if state.lifecycle.phase != BodyPhase::Unused {
            return Err(RequestError::BodyAlreadyConsumed);
        }
        state.lifecycle.phase = BodyPhase::Transferred;
        let stream = match &mut state.source {
            BodySource::Stream(stream) => stream.take().ok_or(RequestError::BodyAlreadyConsumed)?,
            BodySource::Null => Box::pin(futures_util::stream::empty()),
            BodySource::Replayable(bytes) => {
                let bytes = bytes.clone();
                Box::pin(futures_util::stream::once(async move { Ok(bytes) }))
            }
            BodySource::OneShotBytes(bytes) => {
                let bytes = bytes.take().ok_or(RequestError::BodyAlreadyConsumed)?;
                Box::pin(futures_util::stream::once(async move { Ok(bytes) }))
            }
        };
        Ok((stream, state.permit.take()))
    }

    /// Attaches the scheduler permit that remains held while a response body is live.
    pub(crate) fn attach_permit(&mut self, permit: tokio::sync::OwnedSemaphorePermit) {
        self.state
            .lock()
            .expect("body state lock should not be poisoned")
            .permit = Some(permit);
    }
}
