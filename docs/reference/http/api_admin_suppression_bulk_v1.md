# `POST /api/admin/suppression/v1/bulk`
 
 Making a POST request to this endpoint allows the system operator
 to create or update multiple suppression entries in a single request.
 
 Each entry is processed independently; failures for individual entries
 don't prevent other entries from being processed.
 
 The body of the request must be a JSON object; here's an example:
 
 ```json
 {
     "recipients": [
         {
             "recipient": "user1@example.com",
             "type": "non_transactional",
             "source": "complaint",
             "description": "User marked email as spam"
         },
         {
             "recipient": "user2@example.com",
             "type": "transactional",
             "source": "bounce",
             "description": "Hard bounce"
         }
     ]
 }
 ```
 
 and the response will look something like this:
 
 ```json
 {
     "created": 2,
     "updated": 0,
     "errors": []
 }
 ```
 
 ## Request Fields
 
 ### recipients
 
 Required array of suppression entries. Each entry has the same fields as the
 [PUT /api/admin/suppression/v1](api_admin_suppression_v1.md) endpoint:
 
 * `recipient` - Required string. The email address to suppress.
 * `type` - Required string. The type of suppression.
 * `source` - Optional string. How the entry was added.
 * `description` - Optional string. Reason for the suppression.
 * `subaccount_id` - Optional string. Tenant/subaccount identifier.
 
 ## Response Fields
 
 ### created
 
 Integer. Number of new entries created.
 
 ### updated
 
 Integer. Number of existing entries that were updated.
 
 ### errors
 
 Array of error objects for entries that failed to process:
 
 ```json
 {
     "recipient": "invalid-email",
     "message": "Invalid email address format"
 }
 ```
 
 ## Example Usage
 
 ```console
 $ curl -X POST http://127.0.0.1:8000/api/admin/suppression/v1/bulk \
   -H "Content-Type: application/json" \
   -d '{
     "recipients": [
         {"recipient": "user1@example.com", "type": "non_transactional"},
         {"recipient": "user2@example.com", "type": "transactional"}
     ]
   }'
 {"created":2,"updated":0,"errors":[]}
 ```
 
 ## See Also
 
 * [POST /api/admin/suppression/v1](api_admin_suppression_v1.md) - Create/update single entry
 * [POST /api/admin/suppression/v1/bulk/delete](api_admin_suppression_bulk_delete_v1.md) - Bulk delete entries
 