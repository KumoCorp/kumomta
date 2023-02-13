use message::Message;
use timeq::TimeQ;

pub struct Queue {
    queue: TimeQ<Message>,
}
