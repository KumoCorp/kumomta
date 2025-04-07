use maildir::{MailEntry, Maildir};
use std::io;

fn list_mail(mail: MailEntry) {
    println!("Path:         {}", mail.path().display());
    println!("ID:           {}", mail.id());
    println!("Flags:        {}", mail.flags());
    println!("is_draft:     {}", mail.is_draft());
    println!("is_flagged:   {}", mail.is_flagged());
    println!("is_passed:    {}", mail.is_passed());
    println!("is_replied:   {}", mail.is_replied());
    println!("is_seen:      {}", mail.is_seen());
    println!("is_trashed:   {}", mail.is_trashed());
}

fn process_maildirs(maildirs: impl IntoIterator<Item = Maildir>) -> Result<(), io::Error> {
    maildirs.into_iter().try_for_each(|mdir| {
        mdir.list_new()
            .chain(mdir.list_cur())
            .try_for_each(|r| r.map(list_mail))
    })
}

fn main() {
    let rc = match process_maildirs(std::env::args().skip(1).map(Maildir::with_path)) {
        Err(e) => {
            eprintln!("Error: {:?}", e);
            1
        }
        Ok(_) => 0,
    };
    std::process::exit(rc);
}
