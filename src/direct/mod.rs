// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Structs and methods for working with direct messages.
//!
//! Note that direct message access requires a special permissions level above regular read/write
//! access. Your app must be configured to have "read, write, and direct message" access to use any
//! function in this module, even the read-only ones.
//!
//! Although the Twitter website and official apps display DMs as threads between the authenticated
//! user and specific other users, the API does not expose them like this. Separate calls to
//! `received` and `sent` are necessary to fully reconstruct a DM thread. You can partition on
//! `sender_id`/`receiver_id` and sort by their `created_at` to weave the streams together.
//!
//! ## Types
//!
//! * `DirectMessage`/`DMEntities`: A single DM and its associated entities. The `DMEntities`
//!   struct contains information about URLs, user mentions, and hashtags in the DM.
//! * `Timeline`: Effectively the same as `tweet::Timeline`, but gives out `DirectMessage`s
//!   instead.  Returned by functions that traverse collections of DMs.
//! * `ConversationTimeline`/`DMConversations`: This struct and alias are part of the
//!   "conversations" wrapper for loading direct messages into per-recipient threads.
//!
//! ## Functions
//!
//! ### Lookup
//!
//! These functions pull a user's DMs for viewing. `sent` and `received` can be cursored with
//! sub-views like with tweets, so they return a `Timeline` instance that can be navigated at will.
//!
//! * `sent`
//! * `received`
//! * `show`
//! * `conversations`
//!
//! ### Actions
//!
//! These functions are your basic write access to DMs. As a DM does not carry as much metadata as
//! a tweet, the `send` action does not go through a builder struct like with `DraftTweet`.
//!
//! * `send`
//! * `delete`

use std::collections::HashMap;
use std::mem;

use chrono;
use futures_core::Future;
use futures_util::FutureExt;
use hyper::{Body, Request};
use serde::{Deserialize, Deserializer};

use crate::{auth, entities, error, user};
use crate::common::*;

pub use self::fun::*;

mod fun;
mod raw;

///Represents a single direct message.
///
///As a DM has far less metadata than a regular tweet, the structure consequently contains far
///fewer fields. The basic fields are `id`, `text`, `entities`, and `created_at`; everything else
///either refers to the sender or receiver in some manner.
#[derive(Debug)]
pub struct DirectMessage {
    ///Numeric ID for this DM.
    pub id: u64,
    ///UTC timestamp from when this DM was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    ///The text of the DM.
    pub text: String,
    ///Link, hashtag, and user mention information parsed out of the DM.
    pub entities: DMEntities,
    ///The screen name of the user who sent the DM.
    pub sender_screen_name: String,
    ///The ID of the user who sent the DM.
    pub sender_id: u64,
    ///Full information of the user who sent the DM.
    pub sender: Box<user::TwitterUser>,
    ///The screen name of the user who received the DM.
    pub recipient_screen_name: String,
    ///The ID of the user who received the DM.
    pub recipient_id: u64,
    ///Full information for the user who received the DM.
    pub recipient: Box<user::TwitterUser>,
}

impl<'de> Deserialize<'de> for DirectMessage {
    fn deserialize<D>(deser: D) -> Result<DirectMessage, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut raw = raw::RawDirectMessage::deserialize(deser)?;

        for entity in &mut raw.entities.hashtags {
            codepoints_to_bytes(&mut entity.range, &raw.text);
        }
        for entity in &mut raw.entities.symbols {
            codepoints_to_bytes(&mut entity.range, &raw.text);
        }
        for entity in &mut raw.entities.urls {
            codepoints_to_bytes(&mut entity.range, &raw.text);
        }
        for entity in &mut raw.entities.user_mentions {
            codepoints_to_bytes(&mut entity.range, &raw.text);
        }
        if let Some(ref mut media) = raw.entities.media {
            for entity in media.iter_mut() {
                codepoints_to_bytes(&mut entity.range, &raw.text);
            }
        }

        Ok(DirectMessage {
            id: raw.id,
            created_at: raw.created_at,
            text: raw.text,
            entities: raw.entities,
            sender_screen_name: raw.sender_screen_name,
            sender_id: raw.sender_id,
            sender: raw.sender,
            recipient_screen_name: raw.recipient_screen_name,
            recipient_id: raw.recipient_id,
            recipient: raw.recipient,
        })
    }
}

///Container for URL, hashtag, mention, and media information associated with a direct message.
///
///As far as entities are concerned, a DM can contain nearly everything a tweet can. The only thing
///that isn't present here is the "extended media" that would be on the tweet's `extended_entities`
///field. A user can attach a single picture to a DM via the Twitter website or official apps, so
///if that is present, it will be available in `media`. (Note that the functionality to send
///pictures through a DM is unavailable on the public API; only viewing them is possible.)
///
///For all other fields, if the message contains no hashtags, financial symbols ("cashtags"),
///links, or mentions, those corresponding fields will still be present, just empty.
#[derive(Debug, Deserialize)]
pub struct DMEntities {
    ///Collection of hashtags parsed from the DM.
    pub hashtags: Vec<entities::HashtagEntity>,
    ///Collection of financial symbols, or "cashtags", parsed from the DM.
    pub symbols: Vec<entities::HashtagEntity>,
    ///Collection of URLs parsed from the DM.
    pub urls: Vec<entities::UrlEntity>,
    ///Collection of user mentions parsed from the DM.
    pub user_mentions: Vec<entities::MentionEntity>,
    ///If the message contains any attached media, this contains a collection of media information
    ///from it.
    pub media: Option<Vec<entities::MediaEntity>>,
}

/// Helper struct to navigate collections of direct messages by requesting DMs older or newer than
/// certain IDs.
///
/// Using a Timeline to navigate collections of DMs allows you to efficiently cursor through a
/// collection and only load in the messages you need.
///
/// To begin, call a method that returns a `Timeline`, optionally set the page size, and call
/// `start` to load the first page of results:
///
/// ```rust,no_run
/// # use egg_mode::Token;
/// use tokio::runtime::current_thread::block_on_all;
/// # fn main() {
/// # let token: Token = unimplemented!();
/// let mut timeline = egg_mode::direct::received(&token)
///                                     .with_page_size(10);
///
/// for dm in &block_on_all(timeline.start()).unwrap() {
///     println!("<@{}> {}", dm.sender_screen_name, dm.text);
/// }
/// # }
/// ```
///
/// If you need to load the next set of messages, call `older`, which will automatically update the
/// IDs it tracks:
///
/// ```rust,no_run
/// # use egg_mode::Token;
/// use tokio::runtime::current_thread::block_on_all;
/// # fn main() {
/// # let token: Token = unimplemented!();
/// # let mut timeline = egg_mode::direct::received(&token);
/// # block_on_all(timeline.start()).unwrap();
/// for dm in &block_on_all(timeline.older(None)).unwrap() {
///     println!("<@{}> {}", dm.sender_screen_name, dm.text);
/// }
/// # }
/// ```
///
/// ...and similarly for `newer`, which operates in a similar fashion.
///
/// If you want to start afresh and reload the newest set of DMs again, you can call `start` again,
/// which will clear the tracked IDs before loading the newest set of messages. However, if you've
/// been storing these messages as you go, and already know the newest ID you have on hand, you can
/// load only those messages you need like this:
///
/// ```rust,no_run
/// # use egg_mode::Token;
/// use tokio::runtime::current_thread::block_on_all;
/// # fn main() {
/// # let token: Token = unimplemented!();
/// let mut timeline = egg_mode::direct::received(&token)
///                                     .with_page_size(10);
///
/// block_on_all(timeline.start()).unwrap();
///
/// //keep the max_id for later
/// let reload_id = timeline.max_id.unwrap();
///
/// //simulate scrolling down a little bit
/// block_on_all(timeline.older(None)).unwrap();
/// block_on_all(timeline.older(None)).unwrap();
///
/// //reload the timeline with only what's new
/// timeline.reset();
/// block_on_all(timeline.older(Some(reload_id))).unwrap();
/// # }
/// ```
///
/// Here, the argument to `older` means "older than what I just returned, but newer than the given
/// ID". Since we cleared the tracked IDs with `reset`, that turns into "the newest DMs available
/// that were sent after the given ID". The earlier invocations of `older` with `None` do not place
/// a bound on the DMs it loads. `newer` operates in a similar fashion with its argument, saying
/// "newer than what I just returned, but not newer than this given ID". When called like this,
/// it's possible for these methods to return nothing, which will also clear the `Timeline`'s
/// tracked IDs.
///
/// If you want to manually pull messages between certain IDs, the baseline `call` function can do
/// that for you. Keep in mind, though, that `call` doesn't update the `min_id` or `max_id` fields,
/// so you'll have to set those yourself if you want to follow up with `older` or `newer`.
pub struct Timeline {
    ///The URL to request DMs from.
    link: &'static str,
    ///The token used to authenticate requests with.
    token: auth::Token,
    ///Optional set of params to include prior to adding timeline navigation parameters.
    params_base: Option<ParamList<'static>>,
    ///The maximum number of messages to return in a single call. Twitter doesn't guarantee
    ///returning exactly this number, as suspended or deleted content is removed after retrieving
    ///the initial collection of messages.
    pub count: i32,
    ///The largest/most recent DM ID returned in the last call to `start`, `older`, or `newer`.
    pub max_id: Option<u64>,
    ///The smallest/oldest DM ID returned in the last call to `start`, `older`, or `newer`.
    pub min_id: Option<u64>,
}

impl Timeline {
    ///Clear the saved IDs on this timeline.
    pub fn reset(&mut self) {
        self.max_id = None;
        self.min_id = None;
    }

    ///Clear the saved IDs on this timeline, and return the most recent set of messages.
    pub fn start<'s>(
        &'s mut self,
    ) -> impl Future<Output = Result<Response<Vec<DirectMessage>>, error::Error>> + 's {
        self.reset();
        self.older(None)
    }

    ///Return the set of DMs older than the last set pulled, optionally placing a minimum DM ID to
    ///bound with.
    pub fn older<'s>(
        &'s mut self,
        since_id: Option<u64>,
    ) -> impl Future<Output = Result<Response<Vec<DirectMessage>>, error::Error>> + 's {
        let req = self.request(since_id, self.min_id.map(|id| id - 1));
        let loader = make_parsed_future(req);
        loader.map(
            move |resp: Result<Response<Vec<DirectMessage>>, error::Error>| {
                self.map_ids(&resp.as_ref().unwrap().response);
                resp
            },
        )
    }

    ///Return the set of DMs newer than the last set pulled, optionally placing a maximum DM ID to
    ///bound with.
    pub fn newer<'s>(
        &'s mut self,
        max_id: Option<u64>,
    ) -> impl Future<Output = Result<Response<Vec<DirectMessage>>, error::Error>> + 's {
        let req = self.request(self.max_id, max_id);
        let loader = make_parsed_future(req);
        loader.map(
            move |resp: Result<Response<Vec<DirectMessage>>, error::Error>| {
                let resp_unwraped = resp.unwrap();
                self.map_ids(&resp_unwraped.response);
                Ok(resp_unwraped)
            },
        )
    }

    ///Return the set of DMs between the IDs given.
    ///
    ///Note that the range is not fully inclusive; the message ID given by `since_id` will not be
    ///returned, but the message with `max_id` will be returned.
    ///
    ///If the range of DMs given by the IDs would return more than `self.count`, the newest set
    ///of messages will be returned.
    pub fn call(
        &self,
        since_id: Option<u64>,
        max_id: Option<u64>,
    ) -> impl Future<Output = Result<Response<Vec<DirectMessage>>, error::Error>> {
        make_parsed_future(self.request(since_id, max_id))
    }

    ///Helper builder function to set the page size.
    pub fn with_page_size(self, page_size: i32) -> Self {
        Timeline {
            count: page_size,
            ..self
        }
    }

    ///Helper function to construct a `Request` from the current state.
    fn request(&self, since_id: Option<u64>, max_id: Option<u64>) -> Request<Body> {
        let mut params = self.params_base.as_ref().cloned().unwrap_or_default();
        add_param(&mut params, "count", self.count.to_string());

        if let Some(id) = since_id {
            add_param(&mut params, "since_id", id.to_string());
        }

        if let Some(id) = max_id {
            add_param(&mut params, "max_id", id.to_string());
        }

        auth::get(self.link, &self.token, Some(&params))
    }

    ///With the returned slice of DMs, set the min_id and max_id on self.
    fn map_ids(&mut self, resp: &[DirectMessage]) {
        self.max_id = resp.first().map(|status| status.id);
        self.min_id = resp.last().map(|status| status.id);
    }

    ///Create an instance of `Timeline` with the given link and tokens.
    fn new(
        link: &'static str,
        params_base: Option<ParamList<'static>>,
        token: &auth::Token,
    ) -> Self {
        Timeline {
            link: link,
            token: token.clone(),
            params_base: params_base,
            count: 20,
            max_id: None,
            min_id: None,
        }
    }
}

///Wrapper around a collection of direct messages, sorted by their recipient.
///
///The mapping exposed here is from a User ID to a listing of direct messages between the
///authenticated user and that user. For more information, see the docs for [`ConversationTimeline`].
///
///[`ConversationTimeline`]: struct.ConversationTimeline.html
pub type DMConversations = HashMap<u64, Vec<DirectMessage>>;

///Load the given set of conversations into this set.
fn merge(this: &mut DMConversations, conversations: DMConversations) {
    for (id, convo) in conversations {
        let messages = this.entry(id).or_insert(Vec::new());
        let cap = convo.len() + messages.len();
        let old_convo = mem::replace(messages, Vec::with_capacity(cap));

        //ASSUMPTION: these conversation threads are disjoint
        if old_convo.first().map(|m| m.id).unwrap_or(0) > convo.first().map(|m| m.id).unwrap_or(0) {
            messages.extend(old_convo);
            messages.extend(convo);
        } else {
            messages.extend(convo);
            messages.extend(old_convo);
        }
    }
}

/// Helper struct to load both sent and received direct messages, pre-sorting them into
/// conversations by their recipient.
///
/// This timeline loader is meant to get around a limitation of the direct message API endpoints:
/// Twitter only gives endpoints to load all the messages *sent* by the authenticated user, or all
/// the messages *received* by the authenticated user. However, the common user interface for DMs
/// is to separate them by the other account in the conversation. This loader is a higher-level
/// wrapper over the direct `sent` and `received` calls to achieve this interface without library
/// users having to implement it themselves.
///
/// Much like [`Timeline`], simply receiving a `ConversationTimeline` from `conversations` does not
/// load any messages on its own. This is to allow setting the page size before loading the first
/// batch of messages.
///
/// [`Timeline`]: struct.Timeline.html
///
/// `ConversationTimeline` keeps a cache of all the messages its loaded, and updates this during
/// calls to Twitter. Any calls on this timeline that generate a `ConversationFuture` will take
/// ownership of the `ConversationTimeline` so that it can update this cache. The Future will
/// return the `ConversationTimeline` on success. To view the current cache, use the
/// `conversations` field.
///
/// There are two methods to load messages, and they operate by extending the cache by loading
/// messages either older or newer than the extent of the cache.
///
/// **NOTE**: Twitter has different API limitations for sent versus received messages. You can only
/// load the most recent 200 *received* messages through the public API, but you can load up to 800
/// *sent* messages. This can create some strange user-interface if a user has some old
/// conversations, as they can only see their own side of the conversation this way. If you'd like
/// to load as many messages as possible, both API endpoints have a per-call limit of 200. Setting
/// the page size to 200 prior to loading messages allows you to use one function call to load a
/// fairly-complete view of the user's conversations.
///
/// # Example
///
/// ```rust,no_run
/// # use egg_mode::Token;
/// use tokio::runtime::current_thread::block_on_all;
/// # fn main() {
/// # let token: Token = unimplemented!();
/// let mut conversations = egg_mode::direct::conversations(&token);
///
/// // newest() and oldest() consume the Timeline and give it back on success, so assign it back
/// // when it's done
/// conversations = block_on_all(conversations.newest()).unwrap();
///
/// for (id, convo) in &conversations.conversations {
///     let user = block_on_all(egg_mode::user::show(id, &token)).unwrap();
///     println!("Conversation with @{}", user.screen_name);
///     for msg in convo {
///         println!("<@{}> {}", msg.sender_screen_name, msg.text);
///     }
/// }
/// # }
/// ```
pub struct ConversationTimeline {
    sent: Timeline,
    received: Timeline,
    ///The message ID of the most recent sent message in the current conversation set.
    pub last_sent: Option<u64>,
    ///The message ID of the most recent received message in the current conversation set.
    pub last_received: Option<u64>,
    ///The message ID of the oldest sent message in the current conversation set.
    pub first_sent: Option<u64>,
    ///The message ID of the oldest received message in the current conversation set.
    pub first_received: Option<u64>,
    ///The number of messages loaded per API call.
    pub count: u32,
    ///The conversation threads that have been loaded so far.
    pub conversations: DMConversations,
}

impl ConversationTimeline {
    fn new(token: &auth::Token) -> ConversationTimeline {
        ConversationTimeline {
            sent: sent(token),
            received: received(token),
            last_sent: None,
            last_received: None,
            first_sent: None,
            first_received: None,
            count: 20,
            conversations: HashMap::new(),
        }
    }

    fn merge(&mut self, sent: Vec<DirectMessage>, received: Vec<DirectMessage>) {
        self.last_sent = max_opt(self.last_sent, sent.first().map(|m| m.id));
        self.last_received = max_opt(self.last_received, received.first().map(|m| m.id));
        self.first_sent = min_opt(self.first_sent, sent.last().map(|m| m.id));
        self.first_received = min_opt(self.first_received, received.last().map(|m| m.id));

        let sender = sent.first().map(|m| m.sender_id);
        let receiver = received.first().map(|m| m.recipient_id);

        if let Some(me_id) = sender.or(receiver) {
            let mut new_convo = HashMap::new();

            for msg in merge_by(sent, received, |left, right| left.id > right.id) {
                let recipient = if msg.sender_id == me_id {
                    msg.recipient_id
                } else {
                    msg.sender_id
                };

                let thread = new_convo.entry(recipient).or_insert(Vec::new());
                thread.push(msg);
            }

            merge(&mut self.conversations, new_convo);
        }
    }

    ///Builder function to set the number of messages pulled in a single request.
    pub fn with_page_size(self, page_size: u32) -> ConversationTimeline {
        ConversationTimeline {
            count: page_size,
            ..self
        }
    }

    ///Load messages newer than the currently-loaded set, or the newset set if no messages have
    ///been loaded yet. The complete conversation set can be viewed from the `ConversationTimeline`
    ///after it is finished loading.
    pub fn newest(self) -> impl Future<Output = ConversationTimeline> {
        let sent = self.sent.call(self.last_sent, None);
        let received = self.received.call(self.last_received, None);

        self.make_future(sent, received)
    }

    ///Load messages older than the currently-loaded set, or the newest set if no messages have
    ///been loaded. The complete conversation set can be viewed from the `ConversationTimeline`
    ///after it is finished loading.
    pub fn next(self) -> impl Future<Output = ConversationTimeline> {
        let sent = self.sent.call(None, self.first_sent);
        let received = self.received.call(None, self.first_received);

        self.make_future(sent, received)
    }

    fn make_future<S, R>(
        mut self,
        sent: S,
        received: R,
    ) -> impl Future<Output = ConversationTimeline>
    where
        S: Future<Output = Result<Response<Vec<DirectMessage>>, error::Error>>,
        R: Future<Output = Result<Response<Vec<DirectMessage>>, error::Error>>,
    {
        futures_util::future::join(sent, received).map(|(sent, recvd)| {
            self.merge(sent.unwrap().response, recvd.unwrap().response);
            self
        })
    }
}
