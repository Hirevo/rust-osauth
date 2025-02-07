// Copyright 2021 Dmitry Tantsur <dtantsur@protonmail.com>
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Low-level authenticated client.

use std::collections::HashMap;
use std::convert::TryFrom;
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "stream")]
use async_trait::async_trait;
#[cfg(feature = "stream")]
use futures::Stream;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use http::Error as HttpError;
use log::trace;
use reqwest::{Body, Client, Method, Request, RequestBuilder as HttpRequestBuilder, Response, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use static_assertions::assert_eq_size;

#[cfg(feature = "stream")]
use super::stream::{paginated, FetchNext, PaginatedResource};
use super::url as url_utils;
use super::{AuthType, EndpointFilters, Error};

/// A properly typed constant for use with root paths.
///
/// The problem with just using `None` is that the exact type of `Option` is not known.
///
/// An example:
///
/// ```rust,no_run
/// # async fn example() -> Result<(), osauth::Error> {
/// let session = osauth::Session::from_env().await?;
/// let response = session
///     .get(osauth::services::OBJECT_STORAGE, osauth::client::NO_PATH)
///     .send()
///     .await?;
/// # Ok(()) }
/// # #[tokio::main]
/// # async fn main() { example().await.unwrap(); }
/// ```
pub const NO_PATH: Option<&'static str> = None;

/// Authenticated HTTP client.
///
/// Uses `Arc` internally and should be reused when possible by cloning it.
#[derive(Debug, Clone)]
pub struct AuthenticatedClient {
    client: Client,
    auth: Arc<dyn AuthType>,
}

assert_eq_size!(AuthenticatedClient, Option<AuthenticatedClient>);

impl AuthenticatedClient {
    /// Create a new authenticated client.
    pub async fn new<Auth: AuthType + 'static>(
        client: Client,
        auth_type: Auth,
    ) -> Result<AuthenticatedClient, Error> {
        auth_type.refresh(&client).await?;
        Ok(AuthenticatedClient::new_internal(
            client,
            Arc::new(auth_type),
        ))
    }

    #[inline]
    pub(crate) fn new_internal(client: Client, auth: Arc<dyn AuthType>) -> AuthenticatedClient {
        AuthenticatedClient { client, auth }
    }

    /// Get a reference to the authentication type in use.
    #[inline]
    pub fn auth_type(&self) -> &dyn AuthType {
        self.auth.as_ref()
    }

    /// Authenticate a request.
    #[inline]
    async fn authenticate(&self, request: HttpRequestBuilder) -> Result<Request, Error> {
        self.auth
            .authenticate(&self.client, request)
            .await?
            .build()
            .map_err(Error::from)
    }

    /// Get a URL for the requested service.
    #[inline]
    pub async fn get_endpoint(
        &self,
        service_type: &str,
        filters: &EndpointFilters,
    ) -> Result<Url, Error> {
        self.auth
            .get_endpoint(&self.client, service_type, filters)
            .await
    }

    /// Get a reference to the inner (non-authenticated) client.
    #[inline]
    pub fn inner(&self) -> &Client {
        &self.client
    }

    /// Update the authentication.
    ///
    /// # Warning
    ///
    /// Authentication will also be updated for clones of this client, since they share the same
    /// authentication object.
    #[inline]
    pub async fn refresh(&mut self) -> Result<(), Error> {
        self.auth.refresh(&self.client).await
    }

    /// Set a new authentication for this client.
    #[inline]
    pub fn set_auth_type<Auth: AuthType + 'static>(&mut self, auth_type: Auth) {
        self.auth = Arc::new(auth_type);
    }

    /// Set a new internal client implementation.
    #[inline]
    pub fn set_inner(&mut self, client: Client) {
        self.client = client;
    }

    /// Start an authenticated request.
    #[inline]
    pub fn request(&self, method: Method, url: Url) -> RequestBuilder {
        RequestBuilder {
            inner: self.client.request(method, url),
            client: self.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) async fn new_noauth(endpoint: &str) -> AuthenticatedClient {
        use crate::NoAuth;
        AuthenticatedClient::new(Client::new(), NoAuth::new(endpoint).unwrap())
            .await
            .unwrap()
    }
}

impl From<AuthenticatedClient> for Client {
    fn from(value: AuthenticatedClient) -> Client {
        value.client
    }
}

/// A request builder with error handling.
#[derive(Debug)]
#[must_use = "preparing a request is not enough to run it"]
pub struct RequestBuilder {
    inner: HttpRequestBuilder,
    client: AuthenticatedClient,
}

#[derive(Debug, Deserialize)]
struct Message {
    message: Option<String>,
    faultstring: Option<String>,
    title: Option<String>,
    // Ironic legacy format: JSON inside JSON (sigh)
    error_message: Option<String>,
}

impl Message {
    fn convert(self, recursive: bool) -> Option<String> {
        if let Some(value) = self.message.or(self.faultstring).or(self.title) {
            println!("Normal {}", value);
            Some(value)
        } else if recursive {
            if let Some(json) = self.error_message {
                return serde_json::from_str::<Message>(&json).ok().and_then(|msg| {
                    println!("submessage {:?}", msg);
                    msg.convert(false)
                });
            } else {
                None
            }
        } else {
            None
        }
    }
}

impl From<Message> for Option<String> {
    fn from(value: Message) -> Option<String> {
        value.convert(true)
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ErrorResponse {
    Map(HashMap<String, Message>),
    Message(Message),
}

fn extract_message(text: String) -> String {
    serde_json::from_str::<ErrorResponse>(&text)
        .ok()
        .and_then(|body| match body {
            ErrorResponse::Map(map) => map.into_iter().next().and_then(|(_k, v)| v.into()),
            ErrorResponse::Message(msg) => msg.into(),
        })
        .unwrap_or(text)
}

/// Check for OpenStack errors in the response.
pub async fn check(response: Response) -> Result<Response, Error> {
    let status = response.status();
    if status.is_client_error() || status.is_server_error() {
        let message = extract_message(response.text().await?);
        trace!("HTTP request returned {}; error: {}", status, message);
        Err(Error::new(status.into(), message).with_status(status))
    } else {
        trace!(
            "HTTP request to {} returned {}",
            response.url(),
            response.status()
        );
        Ok(response)
    }
}

impl RequestBuilder {
    /// Get a reference to the client.
    #[inline]
    pub fn client(&self) -> &AuthenticatedClient {
        &self.client
    }

    /// Add a body to the request.
    pub fn body<T: Into<Body>>(self, body: T) -> RequestBuilder {
        RequestBuilder {
            inner: self.inner.body(body),
            ..self
        }
    }

    /// Add a header to the request.
    pub fn header<K, V>(self, key: K, value: V) -> RequestBuilder
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
    {
        RequestBuilder {
            inner: self.inner.header(key, value),
            ..self
        }
    }

    /// Add headers to a request.
    pub fn headers(self, headers: HeaderMap) -> RequestBuilder {
        RequestBuilder {
            inner: self.inner.headers(headers),
            ..self
        }
    }

    /// Add a JSON body to the request.
    pub fn json<T: Serialize + ?Sized>(self, json: &T) -> RequestBuilder {
        RequestBuilder {
            inner: self.inner.json(json),
            ..self
        }
    }

    /// Send a query with the request.
    pub fn query<T: Serialize + ?Sized>(self, query: &T) -> RequestBuilder {
        RequestBuilder {
            inner: self.inner.query(query),
            ..self
        }
    }

    /// Override the timeout for the request.
    pub fn timeout(self, timeout: Duration) -> RequestBuilder {
        RequestBuilder {
            inner: self.inner.timeout(timeout),
            ..self
        }
    }

    /// Send the request and receive JSON in response.
    pub async fn fetch<T>(self) -> Result<T, Error>
    where
        T: DeserializeOwned + Send,
    {
        self.send().await?.json::<T>().await.map_err(Error::from)
    }

    /// Send the request and check for errors.
    pub async fn send(self) -> Result<Response, Error> {
        check(self.send_unchecked().await?).await
    }

    /// Send the request without checking for HTTP and OpenStack errors.
    pub async fn send_unchecked(self) -> Result<Response, Error> {
        let req = self.client.authenticate(self.inner).await?;
        trace!("Sending HTTP {} request to {}", req.method(), req.url());
        self.client.client.execute(req).await.map_err(Error::from)
    }

    /// Send the request to the given URL.
    pub(crate) async fn send_unchecked_to(self, url: &Url) -> Result<Response, Error> {
        let mut req = self.client.authenticate(self.inner).await?;
        url_utils::merge(req.url_mut(), url);
        trace!("Sending HTTP {} request to {}", req.method(), req.url());
        self.client.client.execute(req).await.map_err(Error::from)
    }

    #[cfg(test)]
    pub(crate) fn build(self) -> Result<Request, Error> {
        self.inner.build().map_err(From::from)
    }

    /// Send the request and receive JSON in response with pagination.
    ///
    /// See [`ServiceRequestBuilder::fetch_paginated`] for explanation of parameters
    /// and a real world example.
    ///
    /// # Panics
    ///
    /// Will panic during iteration if the request builder has a streaming body.
    ///
    /// [`ServiceRequestBuilder::fetch_paginated`]: crate::ServiceRequestBuilder::fetch_paginated
    #[cfg(feature = "stream")]
    pub async fn fetch_paginated<T>(
        self,
        limit: Option<usize>,
        starting_with: Option<<T as PaginatedResource>::Id>,
    ) -> impl Stream<Item = Result<T, Error>>
    where
        T: PaginatedResource + Unpin,
        <T as PaginatedResource>::Root: Into<Vec<T>> + Send,
    {
        paginated(self, limit, starting_with)
    }

    /// Attempt to clone this request builder.
    pub fn try_clone(&self) -> Option<RequestBuilder> {
        self.inner.try_clone().map(|inner| RequestBuilder {
            inner,
            client: self.client.clone(),
        })
    }
}

#[cfg(feature = "stream")]
#[async_trait]
impl FetchNext for RequestBuilder {
    async fn fetch_next<Q: Serialize + Send, T: DeserializeOwned + Send>(
        &self,
        query: Q,
    ) -> Result<T, Error> {
        let prepared = self
            .try_clone()
            .expect("Builder with a streaming body cannot be used")
            .query(&query);
        prepared.fetch().await
    }
}

#[cfg(test)]
mod test_extract_message {
    use super::extract_message;

    #[test]
    fn test_plain() {
        let msg = "<html><body>I failed</body></html>";
        let result = extract_message(msg.to_string());
        assert_eq!(result, msg);
    }

    #[test]
    fn test_simple_message() {
        let msg = r#"{"message": "I failed"}"#;
        let result = extract_message(msg.to_string());
        assert_eq!(result, "I failed");
    }

    #[test]
    fn test_nested_message() {
        let msg = r#"{"SomethingFailed": {"message": "I failed"}}"#;
        let result = extract_message(msg.to_string());
        assert_eq!(result, "I failed");
    }

    #[test]
    fn test_ironic_message() {
        let msg = r#"{"error_message": {"faultstring": "I failed"}}"#;
        let result = extract_message(msg.to_string());
        assert_eq!(result, "I failed");
    }

    #[test]
    fn test_ironic_legacy() {
        let msg = r#"{"error_message": "{\"faultstring\": \"I failed\"}"}"#;
        let result = extract_message(msg.to_string());
        assert_eq!(result, "I failed");
    }
}
