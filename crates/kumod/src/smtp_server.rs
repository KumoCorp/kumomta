use std::fmt::Debug;
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, ReadHalf,
    WriteHalf,
};
use tracing::instrument;

#[derive(Debug)]
pub struct SmtpServer<T> {
    reader: BufReader<ReadHalf<T>>,
    writer: BufWriter<WriteHalf<T>>,
}

impl<T: AsyncRead + AsyncWrite + Debug> SmtpServer<T> {
    #[instrument]
    pub async fn run(socket: T) -> anyhow::Result<()> {
        let (reader, writer) = tokio::io::split(socket);
        let reader = tokio::io::BufReader::new(reader);
        let writer = tokio::io::BufWriter::new(writer);
        let mut server = SmtpServer { reader, writer };
        server.process().await
    }

    async fn write_message<S: AsRef<str>>(
        &mut self,
        status: u16,
        message: S,
    ) -> anyhow::Result<()> {
        let mut lines = message.as_ref().lines().peekable();
        while let Some(line) = lines.next() {
            let is_last = lines.peek().is_none();
            let sep = if is_last { ' ' } else { '-' };
            let text = format!("{status}{sep}{line}\r\n");
            self.writer.write(text.as_bytes()).await?;
        }
        self.writer.flush().await?;
        Ok(())
    }

    #[instrument]
    async fn process(&mut self) -> anyhow::Result<()> {
        self.write_message(220, "Greetings from KumoMTA\nW00t!\nYeah!")
            .await?;
        loop {
            let mut line = String::new();
            self.reader.read_line(&mut line).await?;
            self.write_message(420, format!("You said:\n{line}"))
                .await?;
        }
    }
}
