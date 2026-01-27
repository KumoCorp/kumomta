# `POST /api/admin/suppression/v1/bulk/delete`
 
 Making a POST request to this endpoint allows the system operator
 to delete multiple suppression entries in a single request.
 
 The body of the request must be a JSON object; here's an example:
 
 ```json
 {
     "recipients": [
         {
             "recipient": "user1@example.com",
             "type": "non_transactional"
         },
         {
             "recipient": "user2@example.com"
         }
     ]
 }
 ```
 
 and the response will look something like this:
 
 ```json
 {
     "deleted": 2,
     "errors": []
 }
 ```
 
 ## Request Fields
 
 ### recipients
 
 Required array of delete requests. Each entry has the following fields:
 
 * `recipient` - Required string. The email address to remove from suppression.
 * `type` - Optional string. The type of suppression to remove.
   If omitted, all types for this recipient are removed.
 * `subaccount_id` - Optional string. Tenant/subaccount identifier.
 
 ## Response Fields
 
 ### deleted
 
 Integer. Total number of entries deleted across all recipients.
 
 ### errors
 
 Array of error objects for entries that failed to process.
 
 ## Example Usage
 
 ```console
 $ curl -X POST http://127.0.0.1:8000/api/admin/suppression/v1/bulk/delete \
   -H "Content-Type: application/json" \
   -d '{
     "recipients": [
         {"recipient": "user1@example.com", "type": "non_transactional"},
         {"recipient": "user2@example.com"}
     ]
   }'
 {"deleted":3,"errors":[]}
 ```
 
 ## See Also
 
 * [DELETE /api/admin/suppression/v1](api_admin_suppression_delete_v1.md) - Delete single entry
 * [POST /api/admin/suppression/v1/bulk](api_admin_suppression_bulk_v1.md) - Bulk create entries
 