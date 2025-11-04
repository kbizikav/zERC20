// SPDX-License-Identifier: GPL-3.0
pragma solidity >=0.7.0 <0.9.0;

/*
    Sonobe's Nova + CycleFold decider verifier.
    Joint effort by 0xPARC & PSE.

    More details at https://github.com/privacy-scaling-explorations/sonobe
    Usage and design documentation at https://privacy-scaling-explorations.github.io/sonobe-docs/

    Uses the https://github.com/iden3/snarkjs/blob/master/templates/verifier_groth16.sol.ejs
    Groth16 verifier implementation and a KZG10 Solidity template adapted from
    https://github.com/weijiekoh/libkzg.
    Additionally we implement the WithdrawLocalNovaDecider contract, which combines the
    Groth16 and KZG10 verifiers to verify the zkSNARK proofs coming from
    Nova+CycleFold folding.
*/


/* =============================== */
/* KZG10 verifier methods */
/**
 * @author  Privacy and Scaling Explorations team - pse.dev
 * @dev     Contains utility functions for ops in BN254; in G_1 mostly.
 * @notice  Forked from https://github.com/weijiekoh/libkzg.
 * Among others, a few of the changes we did on this fork were:
 * - Templating the pragma version
 * - Removing type wrappers and use uints instead
 * - Performing changes on arg types
 * - Update some of the `require` statements 
 * - Use the bn254 scalar field instead of checking for overflow on the babyjub prime
 * - In batch checking, we compute auxiliary polynomials and their commitments at the same time.
 */
contract KZG10Verifier {

    // prime of field F_p over which y^2 = x^3 + 3 is defined
    uint256 public constant BN254_PRIME_FIELD =
        21888242871839275222246405745257275088696311157297823662689037894645226208583;
    uint256 public constant BN254_SCALAR_FIELD =
        21888242871839275222246405745257275088548364400416034343698204186575808495617;

    /**
     * @notice  Performs scalar multiplication in G_1.
     * @param   p  G_1 point to multiply
     * @param   s  Scalar to multiply by
     * @return  r  G_1 point p multiplied by scalar s
     */
    function mulScalar(uint256[2] memory p, uint256 s) internal view returns (uint256[2] memory r) {
        uint256[3] memory input;
        input[0] = p[0];
        input[1] = p[1];
        input[2] = s;
        bool success;
        assembly {
            success := staticcall(sub(gas(), 2000), 7, input, 0x60, r, 0x40)
            switch success
            case 0 { invalid() }
        }
        require(success, "bn254: scalar mul failed");
    }

    /**
     * @notice  Negates a point in G_1.
     * @param   p  G_1 point to negate
     * @return  uint256[2]  G_1 point -p
     */
    function negate(uint256[2] memory p) internal pure returns (uint256[2] memory) {
        if (p[0] == 0 && p[1] == 0) {
            return p;
        }
        return [p[0], BN254_PRIME_FIELD - (p[1] % BN254_PRIME_FIELD)];
    }

    /**
     * @notice  Adds two points in G_1.
     * @param   p1  G_1 point 1
     * @param   p2  G_1 point 2
     * @return  r  G_1 point p1 + p2
     */
    function add(uint256[2] memory p1, uint256[2] memory p2) internal view returns (uint256[2] memory r) {
        bool success;
        uint256[4] memory input = [p1[0], p1[1], p2[0], p2[1]];
        assembly {
            success := staticcall(sub(gas(), 2000), 6, input, 0x80, r, 0x40)
            switch success
            case 0 { invalid() }
        }

        require(success, "bn254: point add failed");
    }

    /**
     * @notice  Computes the pairing check e(p1, p2) * e(p3, p4) == 1
     * @dev     Note that G_2 points a*i + b are encoded as two elements of F_p, (a, b)
     * @param   a_1  G_1 point 1
     * @param   a_2  G_2 point 1
     * @param   b_1  G_1 point 2
     * @param   b_2  G_2 point 2
     * @return  result  true if pairing check is successful
     */
    function pairing(uint256[2] memory a_1, uint256[2][2] memory a_2, uint256[2] memory b_1, uint256[2][2] memory b_2)
        internal
        view
        returns (bool result)
    {
        uint256[12] memory input = [
            a_1[0],
            a_1[1],
            a_2[0][1], // imaginary part first
            a_2[0][0],
            a_2[1][1], // imaginary part first
            a_2[1][0],
            b_1[0],
            b_1[1],
            b_2[0][1], // imaginary part first
            b_2[0][0],
            b_2[1][1], // imaginary part first
            b_2[1][0]
        ];

        uint256[1] memory out;
        bool success;

        assembly {
            success := staticcall(sub(gas(), 2000), 8, input, 0x180, out, 0x20)
            switch success
            case 0 { invalid() }
        }

        require(success, "bn254: pairing failed");

        return out[0] == 1;
    }

    uint256[2] G_1 = [
            17972166172477927580966753801534392926390414011182124296494436899875835459482,
            2218271212538119924126019821815265793826772103153839241786596125668022500874
    ];
    uint256[2][2] G_2 = [
        [
            1154064315684751160495224059341546990274586129569136226981447946927474010357,
            11377576883007562778368087203556526979100510756599572273371116245959322242842
        ],
        [
            14072542809811940477759791225745094028924715801976337561770589948910340746824,
            10297242815337090708932145440564825178130493348230954535537624862979123603686
        ]
    ];
    uint256[2][2] VK = [
        [
            8603883509550290673906136255186266043975262925158834546291170106521717107984,
            19475934799171192024959593067512424885589043002681463988190298614439941798898
        ],
        [
            663358650747201254546605401784794689975749059669526883354234761642933361929,
            6404900493521595653073306930410958023699398323275000203997306403253929532381
        ]
    ];

    

    /**
     * @notice  Verifies a single point evaluation proof. Function name follows `ark-poly`.
     * @dev     To avoid ops in G_2, we slightly tweak how the verification is done.
     * @param   c  G_1 point commitment to polynomial.
     * @param   pi G_1 point proof.
     * @param   x  Value to prove evaluation of polynomial at.
     * @param   y  Evaluation poly(x).
     * @return  result Indicates if KZG proof is correct.
     */
    function check(uint256[2] calldata c, uint256[2] calldata pi, uint256 x, uint256 y)
        public
        view
        returns (bool result)
    {
        //
        // we want to:
        //      1. avoid gas intensive ops in G2
        //      2. format the pairing check in line with what the evm opcode expects.
        //
        // we can do this by tweaking the KZG check to be:
        //
        //          e(pi, vk - x * g2) = e(c - y * g1, g2) [initial check]
        //          e(pi, vk - x * g2) * e(c - y * g1, g2)^{-1} = 1
        //          e(pi, vk - x * g2) * e(-c + y * g1, g2) = 1 [bilinearity of pairing for all subsequent steps]
        //          e(pi, vk) * e(pi, -x * g2) * e(-c + y * g1, g2) = 1
        //          e(pi, vk) * e(-x * pi, g2) * e(-c + y * g1, g2) = 1
        //          e(pi, vk) * e(x * -pi - c + y * g1, g2) = 1 [done]
        //                        |_   rhs_pairing  _|
        //
        uint256[2] memory rhs_pairing =
            add(mulScalar(negate(pi), x), add(negate(c), mulScalar(G_1, y)));
        return pairing(pi, VK, rhs_pairing, G_2);
    }

    function evalPolyAt(uint256[] memory _coefficients, uint256 _index) public pure returns (uint256) {
        uint256 m = BN254_SCALAR_FIELD;
        uint256 result = 0;
        uint256 powerOfX = 1;

        for (uint256 i = 0; i < _coefficients.length; i++) {
            uint256 coeff = _coefficients[i];
            assembly {
                result := addmod(result, mulmod(powerOfX, coeff, m), m)
                powerOfX := mulmod(powerOfX, _index, m)
            }
        }
        return result;
    }

    
}

/* =============================== */
/* Groth16 verifier methods */
/*
    Copyright 2021 0KIMS association.

    * `solidity-verifiers` added comment
        This file is a template built out of [snarkJS](https://github.com/iden3/snarkjs) groth16 verifier.
        See the original ejs template [here](https://github.com/iden3/snarkjs/blob/master/templates/verifier_groth16.sol.ejs)
    *

    snarkJS is a free software: you can redistribute it and/or modify it
    under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    snarkJS is distributed in the hope that it will be useful, but WITHOUT
    ANY WARRANTY; without even the implied warranty of MERCHANTABILITY
    or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public
    License for more details.

    You should have received a copy of the GNU General Public License
    along with snarkJS. If not, see <https://www.gnu.org/licenses/>.
*/

contract Groth16Verifier {
    // Scalar field size
    uint256 constant r    = 21888242871839275222246405745257275088548364400416034343698204186575808495617;
    // Base field size
    uint256 constant q   = 21888242871839275222246405745257275088696311157297823662689037894645226208583;

    // Verification Key data
    uint256 constant alphax  = 14063568825121222987252665438080124095479660142462128354500855392369927740947;
    uint256 constant alphay  = 1036641240717259284197444372481248193261754753028574343243185520684149044826;
    uint256 constant betax1  = 12259590911409424800440662053117587818364164875239061271667150572566487099819;
    uint256 constant betax2  = 8478195762251265088060257755499907823997000555748059046429501634749341767618;
    uint256 constant betay1  = 20283521014826873422244158172279299338628770163953232805445030358540471324375;
    uint256 constant betay2  = 10746896537489857311282020155533382705492447691112948305864240101443447648890;
    uint256 constant gammax1 = 18649285157148027970461862994407973604219964481163434659244824888267758488017;
    uint256 constant gammax2 = 1575722106320445668811215411384293063364624325724406753258425680987761134159;
    uint256 constant gammay1 = 14129827191832349177583864971360710990455293903558908627000487882155733162758;
    uint256 constant gammay2 = 10688633216469393180525166974913388592252465644911013330400707150946468636577;
    uint256 constant deltax1 = 10538651478149458989849339986313735088474632270671711023070564110943669803218;
    uint256 constant deltax2 = 1305467539439163997533585740862968572346371551463775903177786773622991246826;
    uint256 constant deltay1 = 14752496234907181405815200002350513382346626323944746000857471514379525076102;
    uint256 constant deltay2 = 15564852097701493536090688030437847912072338853093368342919193095581144941270;

    
    uint256 constant IC0x = 16470925964922844526144272452358857897429435400960148864339070435075785567037;
    uint256 constant IC0y = 12170208961597153578150762825278078820162468154748617741956008400781146674376;
    
    uint256 constant IC1x = 7148041859745761339562771161905737607012001755472029074209336705730052805594;
    uint256 constant IC1y = 15642484373274387651316660128568892002708498714121892323603671997167900159408;
    
    uint256 constant IC2x = 5834178546109215024398411941298733595653294063582862358268590605716173170518;
    uint256 constant IC2y = 12740132023528854186637332847614297672585006307681833055876119280669727991994;
    
    uint256 constant IC3x = 18321443195448875042022747599700980408316569528120355123410196463664491927660;
    uint256 constant IC3y = 1718436233138701826259926999443652567299767038031389314480542652647727793974;
    
    uint256 constant IC4x = 7473913150804380879788757074170225385461887077111759930854923147020771338586;
    uint256 constant IC4y = 7180637198141293070206788111408548166083563849828992892962837577500111994286;
    
    uint256 constant IC5x = 11672113878546740839564110799701440927314217808941897464555464222256644351845;
    uint256 constant IC5y = 4780446174972379722786725796232773497518745263170105686800669729603860910663;
    
    uint256 constant IC6x = 1361259741414257155635965948690714809684599347428628720845485152381355006836;
    uint256 constant IC6y = 4134633926503052240719929852166058145549319157749782192937220839150819465545;
    
    uint256 constant IC7x = 3753714145606877504105388232999418538188766704806662487407174069892999638681;
    uint256 constant IC7y = 7898746315510687856930157203263113035435631272086588919457676069041516059997;
    
    uint256 constant IC8x = 18867175494692895868569990652989435793184896691092949417499675317798984880064;
    uint256 constant IC8y = 9790612194652675589871499850097817592801285933815355948686218512558899130959;
    
    uint256 constant IC9x = 21840176890382321895346097213393842033029620170337559615367726769571733878446;
    uint256 constant IC9y = 347734815995885976004760571221698272340536067543200861412516072198932778549;
    
    uint256 constant IC10x = 2744815505068003197458552934945454880569022566188302915478087573502135350149;
    uint256 constant IC10y = 8117796691140239184573782934873334547994300817082886670042930542706390261463;
    
    uint256 constant IC11x = 255876999140374681087214171780061753607461668611827920670059801577564608501;
    uint256 constant IC11y = 15047124031783386420494516195339846135233412208553545732923658288575877724223;
    
    uint256 constant IC12x = 19625168480271488153350943671745085454622396829946436303491106600485461753129;
    uint256 constant IC12y = 19455537048051120999486831550332691588970096094032717137968800775852568063377;
    
    uint256 constant IC13x = 6611118053904039452727585935184900995452807020186178986342539014808299773711;
    uint256 constant IC13y = 20090897058190486970572722023777104959421169901973737408212348919915185607693;
    
    uint256 constant IC14x = 7087884275187720600350887158549554976657229211777016848874499067592983712839;
    uint256 constant IC14y = 9280863584342870093719716038865265647673462677548274681095311353684347730970;
    
    uint256 constant IC15x = 11126418992806067601685475449871887633191852148464068543781039696128615962164;
    uint256 constant IC15y = 11919203632824742751700487697301954189151554679504616460980641241981420701422;
    
    uint256 constant IC16x = 13231452217502549202055223922451334356989880875403825200321749259286732566766;
    uint256 constant IC16y = 20761812300072937249594162191600132546947339629045407531938290456890669705109;
    
    uint256 constant IC17x = 7781625551578354179284433079350186279336181170648338878742793825998785609591;
    uint256 constant IC17y = 7869525327501053239669894291720230350888952741042276759119759506512206636140;
    
    uint256 constant IC18x = 3031840039041492154164930496243857043432092490940555408173383132375888864536;
    uint256 constant IC18y = 16694722953814663077385784310839095813470810882846573474651530181587394492242;
    
    uint256 constant IC19x = 18011841559562081152648440278389114917040853452690371215747150930697168182087;
    uint256 constant IC19y = 18943595979152143280332212305232812184225406355861789660841769299183383402831;
    
    uint256 constant IC20x = 9184925000514363284934019665824486831016242247327691855690139318572353686813;
    uint256 constant IC20y = 17205847753425122806061248636465009787715054982386549843306651928152303120587;
    
    uint256 constant IC21x = 21669399656022249473336696087459194366880901464751475911675108767981878540835;
    uint256 constant IC21y = 13084534383452833501016495995657904664451186511261158029311084633774482290207;
    
    uint256 constant IC22x = 3079402924640315399876320894559817896105042335595136587645046517632921558237;
    uint256 constant IC22y = 9296035994135897016814966457519954629671735226450946586327547097062827324494;
    
    uint256 constant IC23x = 16332655857558099711597394438892251302702629464801245598428633786160404680498;
    uint256 constant IC23y = 2721722875561210870082535962538009170879236318359995280450177555943649012238;
    
    uint256 constant IC24x = 15030850026355909361394071169855389198696030158587304931744573710623153208423;
    uint256 constant IC24y = 18541424750459806609197864624469168150921849324025909141217857895761554128462;
    
    uint256 constant IC25x = 10044122889413931546939154958309749648285262400228919562458381847581030136462;
    uint256 constant IC25y = 13748486898811606214505606438604735068063061964581715671247158542040813999174;
    
    uint256 constant IC26x = 11977177825376081129506698899517910457109596763063964064277480811244907055111;
    uint256 constant IC26y = 7474274257790290273878838795568650837504170401818418781140946049384241820531;
    
    uint256 constant IC27x = 20134116003027732632880530045902002539712586909859874338433425063353470844211;
    uint256 constant IC27y = 17776953559950693076300586295569303355309766479776011153669633526806852709442;
    
    uint256 constant IC28x = 15418370983681109197247687268066818030450605772814714267048053057388054793875;
    uint256 constant IC28y = 6461809167947056337390021209473570833942720374402710712309761754674510336594;
    
    uint256 constant IC29x = 11200137099012639758036544668005817634588553838137471099892868217308885070077;
    uint256 constant IC29y = 8492344754319214574997164444074570374678877262156436076324935502447055468789;
    
    uint256 constant IC30x = 730685238715949073442117656181187124922413524534427691324651181936421894902;
    uint256 constant IC30y = 5153528966815724988912235828156092692550387486478544042267469442122345318132;
    
    uint256 constant IC31x = 1275031513569509976592205436233610729484369947701611557664151924287308515374;
    uint256 constant IC31y = 7458979611343353636693520111061388374474695141173771846779666672388892377827;
    
    uint256 constant IC32x = 21570610051072036936725715084326294235819868026659888188397754164838675623763;
    uint256 constant IC32y = 13909159227243763033240170115523508766585848748317367656792904685469944874032;
    
    uint256 constant IC33x = 20350109478254299565864147411409745839972718133428880087853948459724098665966;
    uint256 constant IC33y = 14014796713322005571127696934778955614499105054770704088294324436702434660938;
    
    uint256 constant IC34x = 11799415523392822720367463687837032742503563872266786738289930202219386694323;
    uint256 constant IC34y = 6763380455978038043497259666450801257143870396164527307708379395277567859331;
    
    uint256 constant IC35x = 6980115826784973038763692957791353438946399587170389430936221165628513193719;
    uint256 constant IC35y = 15484962007642697466093473671725189052314197778963856229312292349146122334441;
    
    uint256 constant IC36x = 17940488417694525828611410218946475046298552914316979239360922464607583774708;
    uint256 constant IC36y = 14071331047955268856874312062267778212604530728240670944439078882247328069648;
    
    uint256 constant IC37x = 6713411293083250767106823079959088862327315992004584541912661510834901496621;
    uint256 constant IC37y = 17065257129575317263737237897231845771290635149722638708886926900010218610584;
    
    uint256 constant IC38x = 16324807934983345031569194545252008970178816007675128776881729953840197937984;
    uint256 constant IC38y = 13445774808829626875537677447734806372906637818726729726293231688051283410184;
    
    uint256 constant IC39x = 5585291724201410670395445823900329384420834888418175272261819541048771890525;
    uint256 constant IC39y = 7695358850247368524534294544360178360464248639557024361358206926591005815907;
    
    uint256 constant IC40x = 10076430291848892170614538390797314979197190574468806527458073472193487179304;
    uint256 constant IC40y = 8763142649745998344689103595408853841328016144199209690408279184905704275928;
    
    uint256 constant IC41x = 3380043106814871311635320961806105724149582085620697195911393225319127997845;
    uint256 constant IC41y = 627422818362683052962554627337660326587133462260625006158720376820190789339;
    
    uint256 constant IC42x = 21068571349069871997328150584834815097950116561574339374751256287085846447581;
    uint256 constant IC42y = 3365461984766755248619171375947233040428001064502883272504103581595166585899;
    
    uint256 constant IC43x = 7963039691695288873562244733372612586806805350574757118686929570986532687659;
    uint256 constant IC43y = 19121033962582929376463028996236510582291280593916337543323502986637806109735;
    
    uint256 constant IC44x = 7702691611562229136474019821082995227204241499650715633415402100390495106469;
    uint256 constant IC44y = 9560795373127808420510259076157425960697703761122019864476834311112393651263;
    
    
    // Memory data
    uint16 constant pVk = 0;
    uint16 constant pPairing = 128;

    uint16 constant pLastMem = 896;

    function verifyProof(uint[2] calldata _pA, uint[2][2] calldata _pB, uint[2] calldata _pC, uint[44] calldata _pubSignals) public view returns (bool) {
        assembly {
            function checkField(v) {
                if iszero(lt(v, r)) {
                    mstore(0, 0)
                    return(0, 0x20)
                }
            }
            
            // G1 function to multiply a G1 value(x,y) to value in an address
            function g1_mulAccC(pR, x, y, s) {
                let success
                let mIn := mload(0x40)
                mstore(mIn, x)
                mstore(add(mIn, 32), y)
                mstore(add(mIn, 64), s)

                success := staticcall(sub(gas(), 2000), 7, mIn, 96, mIn, 64)

                if iszero(success) {
                    mstore(0, 0)
                    return(0, 0x20)
                }

                mstore(add(mIn, 64), mload(pR))
                mstore(add(mIn, 96), mload(add(pR, 32)))

                success := staticcall(sub(gas(), 2000), 6, mIn, 128, pR, 64)

                if iszero(success) {
                    mstore(0, 0)
                    return(0, 0x20)
                }
            }

            function checkPairing(pA, pB, pC, pubSignals, pMem) -> isOk {
                let _pPairing := add(pMem, pPairing)
                let _pVk := add(pMem, pVk)

                mstore(_pVk, IC0x)
                mstore(add(_pVk, 32), IC0y)

                // Compute the linear combination vk_x
                
                
                g1_mulAccC(_pVk, IC1x, IC1y, calldataload(add(pubSignals, 0)))
                g1_mulAccC(_pVk, IC2x, IC2y, calldataload(add(pubSignals, 32)))
                g1_mulAccC(_pVk, IC3x, IC3y, calldataload(add(pubSignals, 64)))
                g1_mulAccC(_pVk, IC4x, IC4y, calldataload(add(pubSignals, 96)))
                g1_mulAccC(_pVk, IC5x, IC5y, calldataload(add(pubSignals, 128)))
                g1_mulAccC(_pVk, IC6x, IC6y, calldataload(add(pubSignals, 160)))
                g1_mulAccC(_pVk, IC7x, IC7y, calldataload(add(pubSignals, 192)))
                g1_mulAccC(_pVk, IC8x, IC8y, calldataload(add(pubSignals, 224)))
                g1_mulAccC(_pVk, IC9x, IC9y, calldataload(add(pubSignals, 256)))
                g1_mulAccC(_pVk, IC10x, IC10y, calldataload(add(pubSignals, 288)))
                g1_mulAccC(_pVk, IC11x, IC11y, calldataload(add(pubSignals, 320)))
                g1_mulAccC(_pVk, IC12x, IC12y, calldataload(add(pubSignals, 352)))
                g1_mulAccC(_pVk, IC13x, IC13y, calldataload(add(pubSignals, 384)))
                g1_mulAccC(_pVk, IC14x, IC14y, calldataload(add(pubSignals, 416)))
                g1_mulAccC(_pVk, IC15x, IC15y, calldataload(add(pubSignals, 448)))
                g1_mulAccC(_pVk, IC16x, IC16y, calldataload(add(pubSignals, 480)))
                g1_mulAccC(_pVk, IC17x, IC17y, calldataload(add(pubSignals, 512)))
                g1_mulAccC(_pVk, IC18x, IC18y, calldataload(add(pubSignals, 544)))
                g1_mulAccC(_pVk, IC19x, IC19y, calldataload(add(pubSignals, 576)))
                g1_mulAccC(_pVk, IC20x, IC20y, calldataload(add(pubSignals, 608)))
                g1_mulAccC(_pVk, IC21x, IC21y, calldataload(add(pubSignals, 640)))
                g1_mulAccC(_pVk, IC22x, IC22y, calldataload(add(pubSignals, 672)))
                g1_mulAccC(_pVk, IC23x, IC23y, calldataload(add(pubSignals, 704)))
                g1_mulAccC(_pVk, IC24x, IC24y, calldataload(add(pubSignals, 736)))
                g1_mulAccC(_pVk, IC25x, IC25y, calldataload(add(pubSignals, 768)))
                g1_mulAccC(_pVk, IC26x, IC26y, calldataload(add(pubSignals, 800)))
                g1_mulAccC(_pVk, IC27x, IC27y, calldataload(add(pubSignals, 832)))
                g1_mulAccC(_pVk, IC28x, IC28y, calldataload(add(pubSignals, 864)))
                g1_mulAccC(_pVk, IC29x, IC29y, calldataload(add(pubSignals, 896)))
                g1_mulAccC(_pVk, IC30x, IC30y, calldataload(add(pubSignals, 928)))
                g1_mulAccC(_pVk, IC31x, IC31y, calldataload(add(pubSignals, 960)))
                g1_mulAccC(_pVk, IC32x, IC32y, calldataload(add(pubSignals, 992)))
                g1_mulAccC(_pVk, IC33x, IC33y, calldataload(add(pubSignals, 1024)))
                g1_mulAccC(_pVk, IC34x, IC34y, calldataload(add(pubSignals, 1056)))
                g1_mulAccC(_pVk, IC35x, IC35y, calldataload(add(pubSignals, 1088)))
                g1_mulAccC(_pVk, IC36x, IC36y, calldataload(add(pubSignals, 1120)))
                g1_mulAccC(_pVk, IC37x, IC37y, calldataload(add(pubSignals, 1152)))
                g1_mulAccC(_pVk, IC38x, IC38y, calldataload(add(pubSignals, 1184)))
                g1_mulAccC(_pVk, IC39x, IC39y, calldataload(add(pubSignals, 1216)))
                g1_mulAccC(_pVk, IC40x, IC40y, calldataload(add(pubSignals, 1248)))
                g1_mulAccC(_pVk, IC41x, IC41y, calldataload(add(pubSignals, 1280)))
                g1_mulAccC(_pVk, IC42x, IC42y, calldataload(add(pubSignals, 1312)))
                g1_mulAccC(_pVk, IC43x, IC43y, calldataload(add(pubSignals, 1344)))
                g1_mulAccC(_pVk, IC44x, IC44y, calldataload(add(pubSignals, 1376)))

                // -A
                mstore(_pPairing, calldataload(pA))
                mstore(add(_pPairing, 32), mod(sub(q, calldataload(add(pA, 32))), q))

                // B
                mstore(add(_pPairing, 64), calldataload(pB))
                mstore(add(_pPairing, 96), calldataload(add(pB, 32)))
                mstore(add(_pPairing, 128), calldataload(add(pB, 64)))
                mstore(add(_pPairing, 160), calldataload(add(pB, 96)))

                // alpha1
                mstore(add(_pPairing, 192), alphax)
                mstore(add(_pPairing, 224), alphay)

                // beta2
                mstore(add(_pPairing, 256), betax1)
                mstore(add(_pPairing, 288), betax2)
                mstore(add(_pPairing, 320), betay1)
                mstore(add(_pPairing, 352), betay2)

                // vk_x
                mstore(add(_pPairing, 384), mload(add(pMem, pVk)))
                mstore(add(_pPairing, 416), mload(add(pMem, add(pVk, 32))))


                // gamma2
                mstore(add(_pPairing, 448), gammax1)
                mstore(add(_pPairing, 480), gammax2)
                mstore(add(_pPairing, 512), gammay1)
                mstore(add(_pPairing, 544), gammay2)

                // C
                mstore(add(_pPairing, 576), calldataload(pC))
                mstore(add(_pPairing, 608), calldataload(add(pC, 32)))

                // delta2
                mstore(add(_pPairing, 640), deltax1)
                mstore(add(_pPairing, 672), deltax2)
                mstore(add(_pPairing, 704), deltay1)
                mstore(add(_pPairing, 736), deltay2)


                let success := staticcall(sub(gas(), 2000), 8, _pPairing, 768, _pPairing, 0x20)

                isOk := and(success, mload(_pPairing))
            }

            let pMem := mload(0x40)
            mstore(0x40, add(pMem, pLastMem))

            // Validate that all evaluations ∈ F
            
            checkField(calldataload(add(_pubSignals, 0)))
            
            checkField(calldataload(add(_pubSignals, 32)))
            
            checkField(calldataload(add(_pubSignals, 64)))
            
            checkField(calldataload(add(_pubSignals, 96)))
            
            checkField(calldataload(add(_pubSignals, 128)))
            
            checkField(calldataload(add(_pubSignals, 160)))
            
            checkField(calldataload(add(_pubSignals, 192)))
            
            checkField(calldataload(add(_pubSignals, 224)))
            
            checkField(calldataload(add(_pubSignals, 256)))
            
            checkField(calldataload(add(_pubSignals, 288)))
            
            checkField(calldataload(add(_pubSignals, 320)))
            
            checkField(calldataload(add(_pubSignals, 352)))
            
            checkField(calldataload(add(_pubSignals, 384)))
            
            checkField(calldataload(add(_pubSignals, 416)))
            
            checkField(calldataload(add(_pubSignals, 448)))
            
            checkField(calldataload(add(_pubSignals, 480)))
            
            checkField(calldataload(add(_pubSignals, 512)))
            
            checkField(calldataload(add(_pubSignals, 544)))
            
            checkField(calldataload(add(_pubSignals, 576)))
            
            checkField(calldataload(add(_pubSignals, 608)))
            
            checkField(calldataload(add(_pubSignals, 640)))
            
            checkField(calldataload(add(_pubSignals, 672)))
            
            checkField(calldataload(add(_pubSignals, 704)))
            
            checkField(calldataload(add(_pubSignals, 736)))
            
            checkField(calldataload(add(_pubSignals, 768)))
            
            checkField(calldataload(add(_pubSignals, 800)))
            
            checkField(calldataload(add(_pubSignals, 832)))
            
            checkField(calldataload(add(_pubSignals, 864)))
            
            checkField(calldataload(add(_pubSignals, 896)))
            
            checkField(calldataload(add(_pubSignals, 928)))
            
            checkField(calldataload(add(_pubSignals, 960)))
            
            checkField(calldataload(add(_pubSignals, 992)))
            
            checkField(calldataload(add(_pubSignals, 1024)))
            
            checkField(calldataload(add(_pubSignals, 1056)))
            
            checkField(calldataload(add(_pubSignals, 1088)))
            
            checkField(calldataload(add(_pubSignals, 1120)))
            
            checkField(calldataload(add(_pubSignals, 1152)))
            
            checkField(calldataload(add(_pubSignals, 1184)))
            
            checkField(calldataload(add(_pubSignals, 1216)))
            
            checkField(calldataload(add(_pubSignals, 1248)))
            
            checkField(calldataload(add(_pubSignals, 1280)))
            
            checkField(calldataload(add(_pubSignals, 1312)))
            
            checkField(calldataload(add(_pubSignals, 1344)))
            
            checkField(calldataload(add(_pubSignals, 1376)))
            
            checkField(calldataload(add(_pubSignals, 1408)))
            

            // Validate all evaluations
            let isValid := checkPairing(_pA, _pB, _pC, _pubSignals, pMem)

            mstore(0, isValid)
            
            return(0, 0x20)
        }
    }
}


/* =============================== */
/* Nova+CycleFold Decider verifier */
/**
 * @notice  Computes the decomposition of a `uint256` into num_limbs limbs of bits_per_limb bits each.
 * @dev     Compatible with sonobe::folding-schemes::folding::circuits::nonnative::nonnative_field_to_field_elements.
 */
library LimbsDecomposition {
    function decompose(uint256 x) internal pure returns (uint256[5] memory) {
        uint256[5] memory limbs;
        for (uint8 i = 0; i < 5; i++) {
            limbs[i] = (x >> (55 * i)) & ((1 << 55) - 1);
        }
        return limbs;
    }
}

/**
 * @author PSE & 0xPARC
 * @title  Interface for the WithdrawLocalNovaDecider contract hiding proof details.
 * @dev    This interface enables calling the verifyNovaProof function without exposing the proof details.
 */
interface OpaqueDecider {
    /**
     * @notice  Verifies a Nova+CycleFold proof given initial and final IVC states, number of steps and the rest proof inputs concatenated.
     * @dev     This function should simply reorganize arguments and pass them to the proper verification function.
     */
    function verifyOpaqueNovaProofWithInputs(
        uint256 steps, // number of folded steps (i)
        uint256[4] calldata initial_state, // initial IVC state (z0)
        uint256[4] calldata final_state, // IVC state after i steps (zi)
        uint256[25] calldata proof // the rest of the decider inputs
    ) external view returns (bool);

    /**
     * @notice  Verifies a Nova+CycleFold proof given all the proof inputs collected in a single array.
     * @dev     This function should simply reorganize arguments and pass them to the proper verification function.
     */
    function verifyOpaqueNovaProof(uint256[34] calldata proof) external view returns (bool);
}

/**
 * @author  PSE & 0xPARC
 * @title   WithdrawLocalNovaDecider contract, for verifying Nova IVC SNARK proofs.
 * @dev     This is an askama template which, when templated, features a Groth16 and KZG10 verifiers from which this contract inherits.
 */
contract WithdrawLocalNovaDecider is Groth16Verifier, KZG10Verifier, OpaqueDecider {
    /**
     * @notice  Computes the linear combination of a and b with r as the coefficient.
     * @dev     All ops are done mod the BN254 scalar field prime
     */
    function rlc(uint256 a, uint256 r, uint256 b) internal pure returns (uint256 result) {
        assembly {
            result := addmod(a, mulmod(r, b, BN254_SCALAR_FIELD), BN254_SCALAR_FIELD)
        }
    }

    /**
     * @notice  Verifies a nova cyclefold proof consisting of two KZG proofs and of a groth16 proof.
     * @dev     The selector of this function is "dynamic", since it depends on `z_len`.
     */
    function verifyNovaProof(
        // inputs are grouped to prevent errors due stack too deep
        uint256[9] calldata i_z0_zi, // [i, z0, zi] where |z0| == |zi|
        uint256[4] calldata U_i_cmW_U_i_cmE, // [U_i_cmW[2], U_i_cmE[2]]
        uint256[2] calldata u_i_cmW, // [u_i_cmW[2]]
        uint256[3] calldata cmT_r, // [cmT[2], r]
        uint256[2] calldata pA, // groth16 
        uint256[2][2] calldata pB, // groth16
        uint256[2] calldata pC, // groth16
        uint256[4] calldata challenge_W_challenge_E_kzg_evals, // [challenge_W, challenge_E, eval_W, eval_E]
        uint256[2][2] calldata kzg_proof // [proof_W, proof_E]
    ) public view returns (bool) {

        require(i_z0_zi[0] >= 2, "Folding: the number of folded steps should be at least 2");

        // from gamma_abc_len, we subtract 1. 
        uint256[44] memory public_inputs; 

        public_inputs[0] = 3241676802077219732383012397872680560165228039522745991459365218811386058320;
        public_inputs[1] = i_z0_zi[0];

        for (uint i = 0; i < 8; i++) {
            public_inputs[2 + i] = i_z0_zi[1 + i];
        }

        {
            // U_i.cmW + r * u_i.cmW
            uint256[2] memory mulScalarPoint = super.mulScalar([u_i_cmW[0], u_i_cmW[1]], cmT_r[2]);
            uint256[2] memory cmW = super.add([U_i_cmW_U_i_cmE[0], U_i_cmW_U_i_cmE[1]], mulScalarPoint);

            {
                uint256[5] memory cmW_x_limbs = LimbsDecomposition.decompose(cmW[0]);
                uint256[5] memory cmW_y_limbs = LimbsDecomposition.decompose(cmW[1]);
        
                for (uint8 k = 0; k < 5; k++) {
                    public_inputs[10 + k] = cmW_x_limbs[k];
                    public_inputs[15 + k] = cmW_y_limbs[k];
                }
            }
        
            require(this.check(cmW, kzg_proof[0], challenge_W_challenge_E_kzg_evals[0], challenge_W_challenge_E_kzg_evals[2]), "KZG: verifying proof for challenge W failed");
        }

        {
            // U_i.cmE + r * cmT
            uint256[2] memory mulScalarPoint = super.mulScalar([cmT_r[0], cmT_r[1]], cmT_r[2]);
            uint256[2] memory cmE = super.add([U_i_cmW_U_i_cmE[2], U_i_cmW_U_i_cmE[3]], mulScalarPoint);

            {
                uint256[5] memory cmE_x_limbs = LimbsDecomposition.decompose(cmE[0]);
                uint256[5] memory cmE_y_limbs = LimbsDecomposition.decompose(cmE[1]);
            
                for (uint8 k = 0; k < 5; k++) {
                    public_inputs[20 + k] = cmE_x_limbs[k];
                    public_inputs[25 + k] = cmE_y_limbs[k];
                }
            }

            require(this.check(cmE, kzg_proof[1], challenge_W_challenge_E_kzg_evals[1], challenge_W_challenge_E_kzg_evals[3]), "KZG: verifying proof for challenge E failed");
        }

        {
            // add challenges
            public_inputs[30] = challenge_W_challenge_E_kzg_evals[0];
            public_inputs[31] = challenge_W_challenge_E_kzg_evals[1];
            public_inputs[32] = challenge_W_challenge_E_kzg_evals[2];
            public_inputs[33] = challenge_W_challenge_E_kzg_evals[3];

            uint256[5] memory cmT_x_limbs;
            uint256[5] memory cmT_y_limbs;
        
            cmT_x_limbs = LimbsDecomposition.decompose(cmT_r[0]);
            cmT_y_limbs = LimbsDecomposition.decompose(cmT_r[1]);
        
            for (uint8 k = 0; k < 5; k++) {
                public_inputs[30 + 4 + k] = cmT_x_limbs[k]; 
                public_inputs[35 + 4 + k] = cmT_y_limbs[k];
            }

            bool success_g16 = this.verifyProof(pA, pB, pC, public_inputs);
            require(success_g16 == true, "Groth16: verifying proof failed");
        }

        return(true);
    }

    /**
     * @notice  Verifies a Nova+CycleFold proof given initial and final IVC states, number of steps and the rest proof inputs concatenated.
     * @dev     Simply reorganization of arguments and call to the `verifyNovaProof` function.
     */
    function verifyOpaqueNovaProofWithInputs(
        uint256 steps,
        uint256[4] calldata initial_state,
        uint256[4] calldata final_state,
        uint256[25] calldata proof
    ) public override view returns (bool) {
        uint256[1 + 2 * 4] memory i_z0_zi;
        i_z0_zi[0] = steps;
        for (uint256 i = 0; i < 4; i++) {
            i_z0_zi[i + 1] = initial_state[i];
            i_z0_zi[i + 1 + 4] = final_state[i];
        }

        uint256[4] memory U_i_cmW_U_i_cmE = [proof[0], proof[1], proof[2], proof[3]];
        uint256[2] memory u_i_cmW = [proof[4], proof[5]];
        uint256[3] memory cmT_r = [proof[6], proof[7], proof[8]];
        uint256[2] memory pA = [proof[9], proof[10]];
        uint256[2][2] memory pB = [[proof[11], proof[12]], [proof[13], proof[14]]];
        uint256[2] memory pC = [proof[15], proof[16]];
        uint256[4] memory challenge_W_challenge_E_kzg_evals = [proof[17], proof[18], proof[19], proof[20]];
        uint256[2][2] memory kzg_proof = [[proof[21], proof[22]], [proof[23], proof[24]]];

        return this.verifyNovaProof(
            i_z0_zi,
            U_i_cmW_U_i_cmE,
            u_i_cmW,
            cmT_r,
            pA,
            pB,
            pC,
            challenge_W_challenge_E_kzg_evals,
            kzg_proof
        );
    }

    /**
     * @notice  Verifies a Nova+CycleFold proof given all proof inputs concatenated.
     * @dev     Simply reorganization of arguments and call to the `verifyNovaProof` function.
     */
    function verifyOpaqueNovaProof(uint256[34] calldata proof) public override view returns (bool) {
        uint256[4] memory z0;
        uint256[4] memory zi;
        for (uint256 i = 0; i < 4; i++) {
            z0[i] = proof[i + 1];
            zi[i] = proof[i + 1 + 4];
        }

        uint256[25] memory extracted_proof;
        for (uint256 i = 0; i < 25; i++) {
            extracted_proof[i] = proof[9 + i];
        }

        return this.verifyOpaqueNovaProofWithInputs(proof[0], z0, zi, extracted_proof);
    }
}