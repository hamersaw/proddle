@0xa74d94a500622841;

interface Proddle {
    getMeasurements @0 (measurements :List(Measurement)) -> (measurements :List(Measurement));
    getOperations @1 (bucketHashes :List(BucketHash)) -> (operationBuckets :List(OperationBucket));
    sendMeasurements @2 (measurements: List(Result)) -> ();
}

struct BucketHash {
    bucket @0 :UInt64;
    hash @1 :UInt64;
}

struct Parameter {
    name @0 :Text;
    value @1 :Text;
}

struct Measurement {
    timestamp @0 :Int64;
    name @1 :Text;
    version @2 :Int32;
    parameters @3: List(Parameter);
    dependencies @4 :List(Text);
    content @5 :Text;
}

struct Operation {
    timestamp @0 :Int64;
    measurement @1 :Text;
    domain @2 :Text;
    url @3 :Text;
    parameters @4 :List(Parameter);
    tags @5 :List(Text);
}

struct OperationBucket {
    bucket @0: UInt64;
    operations @1 :List(Operation);
}

struct Result {
    jsonString @0: Text;
}
