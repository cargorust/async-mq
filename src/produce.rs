// SPDX-License-Identifier: APACHE-2.0 AND MIT
//! [ProducerBuilder], [Producer] structs, and [ProducerExt] traits
//!
//! [ProducerBuilder]: struct.ProducerBuilder.html
//! [Producer]: struct.Producer.html
//! [ProducerExt]: trait.ProducerExt.html
use async_trait::async_trait;
use futures_util::stream::StreamExt;
use lapin;

/// A [Producer] builder.
///
/// [Producer]: struct.Producer.html
#[derive(Clone)]
pub struct ProducerBuilder {
    conn: crate::Connection,
    ex: String,
    queue: String,
    queue_opts: lapin::options::QueueDeclareOptions,
    field_table: lapin::types::FieldTable,
    tx_props: lapin::BasicProperties,
    tx_opts: lapin::options::BasicPublishOptions,
    rx_opts: lapin::options::BasicConsumeOptions,
    ack_opts: lapin::options::BasicAckOptions,
    extension: Box<dyn crate::ProducerExt + Send>,
}

impl ProducerBuilder {
    pub fn new(conn: crate::Connection) -> Self {
        Self {
            conn,
            ex: String::from(""),
            queue: String::from(""),
            queue_opts: lapin::options::QueueDeclareOptions::default(),
            field_table: lapin::types::FieldTable::default(),
            tx_props: lapin::BasicProperties::default(),
            tx_opts: lapin::options::BasicPublishOptions::default(),
            rx_opts: lapin::options::BasicConsumeOptions::default(),
            ack_opts: lapin::options::BasicAckOptions::default(),
            extension: Box::new(crate::produce::DebugPrinter {}),
        }
    }
    pub fn exchange(&mut self, exchange: String) -> &mut Self {
        self.ex = exchange;
        self
    }
    pub fn queue(&mut self, queue: String) -> &mut Self {
        self.queue = queue;
        self
    }
    /// Override the default [DebugPrinter] [ProducerExt] trait object.
    ///
    /// [DebugPrinter]: struct.DebugPrinter.html
    /// [ProducerExt]: trait.ProducerExt.html
    pub fn with_ext(&mut self, extension: Box<dyn crate::ProducerExt + Send>) -> &mut Self {
        self.extension = extension;
        self
    }
    pub async fn build(&self) -> lapin::Result<Producer> {
        let tx = match self
            .conn
            .channel(
                &self.queue,
                self.queue_opts.clone(),
                self.field_table.clone(),
            )
            .await
        {
            Ok((ch, _)) => ch,
            Err(err) => return Err(err),
        };
        let opts = lapin::options::QueueDeclareOptions {
            exclusive: true,
            auto_delete: true,
            ..self.queue_opts.clone()
        };
        let (rx, q) = match self.conn.channel("", opts, self.field_table.clone()).await {
            Ok((ch, q)) => (ch, q),
            Err(err) => return Err(err),
        };
        let consume = match rx
            .basic_consume(
                &q,
                "producer",
                self.rx_opts.clone(),
                self.field_table.clone(),
            )
            .await
        {
            Ok(c) => c,
            Err(err) => return Err(err),
        };
        Ok(Producer {
            tx,
            rx,
            consume,
            ex: self.ex.clone(),
            queue: self.queue.clone(),
            tx_props: self.tx_props.clone(),
            rx_props: self.tx_props.clone().with_reply_to(q.name().clone()),
            tx_opts: self.tx_opts.clone(),
            ack_opts: self.ack_opts.clone(),
            extension: self.extension.clone(),
        })
    }
}

/// A zero-cost message producer over [lapin::Channel].
///
/// [lapin::Channel]: https://docs.rs/lapin/latest/lapin/struct.Channel.html
pub struct Producer {
    tx: lapin::Channel,
    rx: lapin::Channel,
    consume: lapin::Consumer,
    ex: String,
    queue: String,
    tx_props: lapin::BasicProperties,
    rx_props: lapin::BasicProperties,
    tx_opts: lapin::options::BasicPublishOptions,
    ack_opts: lapin::options::BasicAckOptions,
    extension: Box<dyn crate::ProducerExt + Send>,
}

impl Producer {
    /// Override the default [DebugPrinter] [ProducerExt] trait object.
    ///
    /// [DebugPrinter]: struct.DebugPrinter.html
    /// [ProducerExt]: trait.ProducerExt.html
    pub fn with_ext(&mut self, extension: Box<dyn crate::ProducerExt + Send>) -> &mut Self {
        self.extension = extension;
        self
    }
    pub async fn rpc(&mut self, msg: Vec<u8>) -> lapin::Result<()> {
        self.tx
            .basic_publish(
                &self.ex,
                &self.queue,
                self.tx_opts.clone(),
                msg,
                self.rx_props.clone(),
            )
            .await?;
        if let Some(msg) = self.consume.next().await {
            match msg {
                Ok(msg) => self.recv(msg).await?,
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }
    pub async fn publish(&mut self, msg: Vec<u8>) -> lapin::Result<()> {
        self.tx
            .basic_publish(
                &self.ex,
                &self.queue,
                self.tx_opts.clone(),
                msg,
                self.tx_props.clone(),
            )
            .await
    }
    async fn recv(&mut self, msg: lapin::message::Delivery) -> lapin::Result<()> {
        let delivery_tag = msg.delivery_tag;
        if let Ok(()) = self.extension.recv(msg.data).await {
            if let Err(err) = self.rx.basic_ack(delivery_tag, self.ack_opts.clone()).await {
                return Err(err);
            }
        }
        Ok(())
    }
}

/// A trait to extend the [Producer] capability.
///
/// [Producer]: struct.Producer.html
#[async_trait]
pub trait ProducerExt {
    async fn recv(&mut self, msg: Vec<u8>) -> lapin::Result<()>;
    fn box_clone(&self) -> Box<dyn ProducerExt + Send>;
}

// https://users.rust-lang.org/t/solved-is-it-possible-to-clone-a-boxed-trait-object/1714/6
impl Clone for Box<dyn ProducerExt + Send> {
    fn clone(&self) -> Box<dyn ProducerExt + Send> {
        self.box_clone()
    }
}

/// A default [ProducerExt] implementor that prints out the received
/// message to `stderr`.
///
/// [ProducerExt]: trait.ProducerExt.html
#[derive(Clone)]
pub struct DebugPrinter;

#[async_trait]
impl ProducerExt for DebugPrinter {
    async fn recv(&mut self, msg: Vec<u8>) -> lapin::Result<()> {
        eprintln!("{:?}", msg);
        Ok(())
    }
    fn box_clone(&self) -> Box<dyn ProducerExt + Send> {
        Box::new((*self).clone())
    }
}
