# Processes are listed only while the Panel is open

Listing every process is the most expensive thing blinkengraphs could do each second. The user explicitly chose not to keep a background record of who caused each spike. A Process Snapshot is therefore taken only while the Panel is visible, and History never stores process names. A past spike can be explained only through Metrics and Probes, not by process. Re-adding a background log would change both the storage schema and the idle CPU budget.
