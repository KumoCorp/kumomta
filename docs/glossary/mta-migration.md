# MTA migration

MTA migration is moving production mail flow from one MTA to another: reproducing routing policy, shaping rules, signing, and integrations on the new platform, then shifting traffic gradually so reputation and volume history carry over. Suppression lists and engagement history must move with the mail; losing them on the cutover recreates years-old list problems overnight. Because sending behavior is reputation-bearing, migrations are done incrementally (by traffic share, tenant, or provider), never as a big-bang cutover.
